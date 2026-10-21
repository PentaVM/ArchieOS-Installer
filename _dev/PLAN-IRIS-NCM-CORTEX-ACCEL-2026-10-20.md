# Plan: IRIS sobre la arquitectura NCM y un "megakernel" para los modelos de ArchieOS

Estado: propuesta para aprobar. Nada de esto está implementado.

## 0. Lo que hay hoy (revisado en el código)

**IRIS Beta 2.1** (`archie-iris-runtime`, Rust/candle, rama `claude/laughing-lamport-5af9cs`
del clon en `/home/archie/iris/ArchieOS-Nvidia`; entrenamiento en `archie_iris_foundry`):

| Parte | Qué es | ¿Se toca? |
|---|---|---|
| V-JEPA 2.1 ViT-B/16 | codificador de imagen, RoPE 3-D | no: pesos congelados |
| SigLIP2 visión + texto | segundo codificador y frases | no |
| Proyecciones + fusión | capas de fusión con **MoE disperso** (top-k + experto compartido, capacidad fija) | no |
| Decodificador DETR | detección en paralelo, embeddings de identidad | no |
| `ActionMemoryHead` | memoria T1–T4, **clasifica** 8 controles (bins de mouse, teclas) | se le **agrega** una cabeza nueva al lado |

No hay difusión en ninguna parte de IRIS (ni en el runtime ni en Foundry). Lo que cambia es
la salida de acción: hoy 8 controles por llamada; con NCM, un plan de acción grueso→fino.

**Cortex** (`cortex/`, C++): daemon sin privilegios que recibe una instantánea del grafo y
devuelve un `ContextBundle`; no ejecuta, no tiene autoridad, no usa la GPU. Se respeta.

**Hoy en la RTX 3070**: entrenamiento LNCM (GPU), LLM de Wanda (CPU), IRIS caído por OOM.

## 1. IRIS → NCM sin perder entrenamiento

Regla: el tronco de IRIS (V-JEPA, SigLIP2, fusión MoE, DETR, memoria) queda con **los mismos
pesos, mismo SHA-256**. Una salida nueva no se obtiene sin entrenar *algo*; lo que se entrena
es solo una cabeza nueva, con el tronco congelado, y la cabeza actual sigue disponible.

1. **Rebase de código**: llevar `archie-iris-runtime` a `main` de ArchieOS-Nvidia (hoy vive en
   otra rama) y unificar su `nn.rs` con el del LNCM en un crate común (`archie-nn`): las ops
   fusionadas y cualquier kernel nuevo sirven a los dos. Prueba: `tests/parity.rs` de IRIS
   (error relativo ≤ 1e-3 contra la referencia PyTorch) y el pipeline del LNCM, sin cambios.
2. **Tokenizador de acciones** (como el VQ del LNCM): VQ residual multiescala sobre
   trayectorias de control de 64 pasos (niveles 4/16/64): niveles gruesos = intención
   ("ir al botón X", "escribir"), finos = cada control.
3. **Cabeza IRIS-NCM**: generador prefix-LM cuyo "prompt" son los tokens fusionados de IRIS +
   su memoria T1–T4; genera los niveles en 3 pasadas → 64 controles en lugar de 8 por llamada.
4. **Entrenamiento barato**: tronco congelado (sin gradiente, en bf16), datos de las fuentes de
   acción existentes (`act_sources`, grabaciones) y la cabeza actual como maestra
   (autodestilación); cabe en la 3070.
5. **A/B con los mismos pesos**: misma IRIS, cabeza vieja vs IRIS-NCM, en las evaluaciones
   retenidas (puntero GUI, composite v3). Se promueve solo si iguala o supera; si no, queda
   apagada. Toda acción sigue pasando por Cerebra → policy → Stage como hoy.
6. **Puente IRIS ↔ Wanda**: el LNCM de Wanda recibe "conceptos de escena" de IRIS (identidad,
   texto leído) como un canal más de su prompt. Comparten generador y kernels, no codec.

## 2. "Megakernel" en ArchieOS: dónde y en qué orden

Un megakernel real (Hazy Research "No Bubbles", Mirage MPK) funde **un** modelo en un kernel
persistente. Fundir **varios** modelos exige que todos usen los mismos kernels propios: el LLM
corre en llama.cpp (ggml) y no entra sin reescribirlo. Por eso, por etapas, midiendo cada una:

| Etapa | Qué | Dónde |
|---|---|---|
| M0 | Medir: tiempo GPU por modelo, lanzamientos/s, burbujas, VRAM (nsys) | todas |
| M1 | **Un dueño de la GPU**: un proceso `archie-accel` carga IRIS + LNCM (ambos candle) en un contexto CUDA, un pool de memoria, un stream por modelo con prioridad (voz/reflejo > IRIS > Wanda > lote LLM > entrenamiento). El Cortex le pasa prioridades (foco/atención); no ejecuta | `archie-model-runtime` |
| M2 | CUDA graphs por paso de cada modelo (frame de IRIS, nivel del LNCM): sin costo de lanzamiento | candle |
| M3 | Batching continuo entre pedidos del mismo modelo (varias sesiones IRIS, varias preguntas) | runtime |
| M4 | **Megakernel por modelo**: primero el LNCM small (22 M, d 384, 6 capas), luego la cabeza de acción de IRIS; cola de instrucciones y contadores por tarea, pesos en streaming a memoria compartida | kernels CUDA propios |
| M5 | Megakernel cruzado (varios modelos, una cola de tareas) solo si M0–M4 muestran SMs ociosos entre modelos | investigación |

M1–M3 dan casi toda la ganancia de "más salida con los mismos recursos" (sin contención,
sin lanzamientos, lotes llenos). M4–M5 se justifican solo con las mediciones de M0.

## 3. Adaptar distros Linux al "kernel Archie" (sin módulo propio)

Un módulo de kernel propio no hace falta y es riesgoso (firma, mantenimiento, seguridad). Lo
mismo se logra en espacio de usuario + eBPF, portable entre distros:

- **cgroups v2 por servicio** (IRIS, LLM, Wanda, entrenamiento) con límites de RAM y
  `oom_score_adj`: hoy, una compilación mató a IRIS por OOM; con cgroups no pasa.
- **sched_ext** (eBPF, kernel ≥ 6.12): planificador que prioriza hilos interactivos (voz,
  reflejo) sobre lotes. WSL usa kernel 5.15: ahí no hay sched_ext ni MPS; en la ISO sí.
- **NVIDIA**: modo persistencia, MPS en Linux nativo, límite de potencia (ya hay guardián
  térmico); pesos con `mlock`/hugepages para que no se paginen.
- Todo como units de systemd + un paquete `archie-host`, igual en Arch, Debian o Fedora.

## 4. Orden propuesto

1. Terminar el entrenamiento LNCM en curso; reiniciar IRIS; LLM de Wanda a GPU.
2. M0 (medir) + rebase de `archie-iris-runtime` + `archie-nn` común (sin cambiar pesos).
3. M1–M3 (`archie-accel`) y cgroups.
4. Tokenizador de acciones + cabeza IRIS-NCM + A/B.
5. M4 (megakernel del LNCM), con números de M0 delante.

## 5. Decisiones que necesito

- ¿La cabeza NCM **convive** con la actual (recomendado) o la reemplaza si gana?
- ¿"Difusión" era la salida de acción? Si querías agregar una política por difusión, es otra
  cabeza (también entrenable con el tronco congelado); NCM la hace en 3 pasadas en vez de ~10–50.
- ¿Empiezo por M0 + rebase (no toca pesos ni entrenamiento) mientras termina el LNCM?
