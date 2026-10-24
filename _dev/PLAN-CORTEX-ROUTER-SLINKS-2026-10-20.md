# Plan de optimización: IRIS rápido, router de memoria y expertos, y S-LINKs entre IRIS, Wanda y el LLM, guiados por Cortex

Fecha: 2026-10-20. Estado: propuesta. Nada de esto está implementado todavía.
Reemplaza las preguntas abiertas de `PLAN-IRIS-NCM-CORTEX-ACCEL-2026-10-20.md`: el dueño ya las respondió (ver §1).

Cada número lleva su origen:
- **medido**: salió de una medición en la RTX 3070.
- **release**: leído del archivo de pesos `iris-beta21-step132000.pt`.
- **código**: leído en el código fuente.
- **estimado**: cálculo propio sin medir, que hay que confirmar en F0.

## 0. Resumen

1. **IRIS no está lento por la GPU, está lento por cómo se le da el trabajo.**
   - Con bf16 en la 3070 (**medido**): 517 ms en modo abierto y 138 ms en modo enfocado.
   - Eso da unos 4 TFLOPS efectivos (**estimado**), alrededor del 10 % del pico de la tarjeta.
   - Las causas que se ven en el código:
     - el MoE rutea en la CPU y sincroniza la GPU unas 65 veces por capa;
     - la atención guarda en memoria la matriz completa de puntajes;
     - todo el archivo de pesos sube a la GPU, incluida la torre de texto;
     - el modo abierto repite la fusión 17 veces sobre la misma imagen.
2. **Primero, arreglos exactos** (mismos pesos, mismo resultado; los verifica la prueba de paridad que ya existe). Después, kernels (flash-attention). Al final, el router de ejecución.
3. **El "megakernel" en el sentido del dueño** es `archie-accel`: un dueño único de la GPU para los modelos candle (IRIS, LNCM y sus cabezas).
   - Decide cada tick:
     - qué pasadas corren;
     - en qué lote;
     - con qué prioridad;
     - qué expertos y qué pesos viven en GPU, RAM o SSD.
   - Toma las ideas de Kimi K3 en vLLM que sirven aquí:
     - el experto compartido en un stream lateral;
     - prefetch de pesos mientras la GPU hace otra cosa;
     - un plan estático por tamaño de lote;
     - nada de preparación de entrada ni de manejo de estado dentro del bucle caliente.
4. **Los puntos de foco** (pantallas, cámaras) reciben **presupuesto de cómputo y de memoria**, no de contexto.
   - Lo que IRIS ve entra como **estado de tamaño fijo**, actualizado por deltas.
   - Al contexto del LLM solo entra lo que la tarea pide, con el presupuesto de tokens que fija Cortex.
5. **Puente OCR → LNCM**:
   - IRIS lee el texto solo donde la escena cambió (cabeza de lectura `read_decoder`, ya entrenada en la release y aún sin portar a Rust).
   - Las líneas nuevas van por un S-LINK, en lote, a una actualización de estado del LNCM de Wanda. No hay compactación.
   - Después, la cabeza de lectura se reentrena para emitir **ids del códec de Wanda**, y esos ids entran directo a la capa de embedding del LNCM.
6. **S-LINKs**: canales tipados y versionados.
   - Un modelo publica su estado intermedio, por ejemplo el nivel grueso de Wanda (la intención) en cuanto lo tiene.
   - Los demás leen la última versión sin esperar.
   - El post-procesado final se hace en una tanda conjunta: la latencia total pasa a ser el máximo de los modelos, no la suma.
   - Llevan datos, nunca autoridad: toda acción sigue Cerebra → Policy → lease → Stage.
7. **LNCM en lugar de LoRA**:
   - Las skills de IRIS pasan a ser **estado**, es decir vectores de skill dentro del prompt de una cabeza LNCM de acción, y no deltas de pesos.
   - Así varias sesiones con skills distintas comparten el mismo lote.
   - Como IRIS no tiene difusión (verificado), NCM reemplaza la generación paso a paso: acciones y lectura.
8. **Cortex aconseja y no ejecuta**, y su contrato ya lo exige: `predict_process` "no puede cambiar política de scheduler, prioridad ni grants".
   - Aporta el foco (`FocusState`), predicciones de latencia y memoria, y propuestas `CandidateSchedulerChange`.
   - `archie-accel` las aplica dentro del sobre que aprueba Policy.

## 1. Decisiones del dueño (2026-10-20)

| Tema | Decisión |
|---|---|
| Difusión | Nunca. Donde haría falta generación iterativa, NCM (grueso → fino, pocas pasadas en paralelo) |
| Skills de IRIS | Iban a ser LoRA (`archie_iris_foundry/lora.py`); pasan a LNCM |
| "Megakernel" | Orquestación a nivel OS: ruteo de expertos, estado SSD/RAM/CPU/GPU, presupuesto por punto de foco, como el trabajo de vLLM en Kimi K3 |
| Conexión entre IAs | S-LINKs: enlaces de software dedicados que pasan contexto a mitad del procesamiento, con post-procesado conjunto |
| Puente OCR | El experto de lectura de IRIS va a la primera capa del LNCM de Wanda, en lote, actualizando estado en vez de compactar |

## 2. Qué hay hoy

### 2.1 IRIS Beta 2.1 por dentro

Configuración (**release**):
- canvas 512, es decir una grilla de 32 × 32 = 1024 tokens;
- fusión de ancho 512 con 8 cabezas;
- 6 capas de fusión, cada una con un MoE de 32 expertos top-2 (512→2048→512), capacidad 1.5 y un experto compartido de 512→1024→512;
- decodificador DETR de 6 capas con 100 consultas;
- hasta 32 tokens de texto por frase y 16 frases por fila;
- `act_chunk` 8, y las cabezas de máscara, keypoints, lectura y temporal entrenadas.

Parámetros por módulo (**release**, contados del state dict; 707 M en total, 1349 MiB en bf16):

| Módulo | Parámetros | bf16 | ¿Lo usa el runtime Rust? |
|---|---:|---:|---|
| Expertos del MoE (6 × 32) | 403.1 M | 769 MiB | sí |
| SigLIP2 visión | 93.2 M | 178 MiB | sí |
| V-JEPA 2.1 ViT-B | 86.8 M | 166 MiB | sí |
| Decodificador + cajas + puntajes | 39.4 M | 75 MiB | sí |
| Fusión: atención, normas, FFN de texto | 31.5 M | 60 MiB | sí |
| Cabeza de acción (`act`) | 14.6 M | 28 MiB | sí (`/act`) |
| Temporal (mixer + predictor) | 12.4 M | 23 MiB | **no**, pero se sube a la GPU |
| **Lectura (`read_decoder`)** | 8.6 M | 16 MiB | **no**, pero se sube a la GPU |
| Router + experto compartido | 6.4 M | 12 MiB | sí |
| Instancias, proyecciones, máscara, keypoints y otras pequeñas | ~11.2 M | ~21 MiB | la mayoría sí |

Cada experto es una unidad de **2.1 M parámetros, 4.0 MiB en bf16**. Hay 192 en total. Fuera de la release, la torre de texto SigLIP2 ocupa ~282 M parámetros (1.13 GB en f32 según `archie-iris-text-pack`), unos 565 MiB más en bf16 **en la GPU**, aunque solo corre para frases nuevas.

### 2.2 Mediciones en la RTX 3070

`IRIS-RTX3070-BENCH.md`, 2026-10-16, bf16 (**medido**):

| Caso | Mediana | fps |
|---|---:|---:|
| open (`mode=all`, 256 frases = 17 filas) | 517.0 ms | 1.93 |
| focused16 / focused4 (1 fila) | 138.4 / 139.6 ms | 7.2 |
| En vivo, con barrido abierto cada 2 s | — | 5.5 |

- VRAM propia de IRIS: ~3.0–3.2 GiB (**medido** como total menos el escritorio).
- Bug en producción: codificar 2 o más frases nuevas en lote falla en CUDA con "matmul is only supported for contiguous tensors".
- La GPU se cayó del sistema (`CM_PROB_PHANTOM`) al empezar la corrida fp32, con Wanda usando la GPU.

**Derivado** de las medianas:
- cada fila extra del modo abierto cuesta (517 − 138) / 16 ≈ **24 ms**;
- el modo enfocado gasta **~114 ms** en codificar la imagen y prepararla, más ~24 ms de una fila de fusión y decodificación.

### 2.3 Cuentas: dónde está el techo (estimado)

FLOPs por frame contando solo las multiplicaciones de matrices (**estimado**):

| Parte | FLOPs |
|---|---|
| V-JEPA (12 capas) | ≈ 213 G |
| SigLIP visión (12 capas) | ≈ 213 G |
| Fusión, por fila | ≈ 104 G (de eso, el MoE ≈ 64 G) |
| Decodificador y puntajes, por fila | ≈ 9 G |

| Caso | FLOPs | Tiempo medido | Rendimiento efectivo |
|---|---:|---:|---:|
| focused (1 fila) | ≈ 0.54 T | 138 ms | ≈ 3.9 TFLOPS |
| open (17 filas) | ≈ 2.35 T | 517 ms | ≈ 4.5 TFLOPS |
| solo los codificadores (derivado) | ≈ 0.43 T | ≈ 114 ms | ≈ 3.7 TFLOPS |

Por ficha técnica, la 3070 llega a ~40 TFLOPS densos en bf16 con acumulación fp32 (no medido aquí).
- Dos ViT-B a 1024 tokens son GEMMs grandes, de las que cuBLAS suele rendir bien. Que tarden ~114 ms indica que el tiempo se va en conversiones, copias, sincronizaciones y preparación, no en cálculo.
- **F0 tiene que confirmarlo con un perfil**, antes de escribir un solo kernel.

### 2.4 Presupuesto de VRAM en servicio (8 GiB)

| Ocupante | VRAM | Origen |
|---|---:|---|
| Escritorio Windows (WDDM) | 1.3–1.4 GiB | medido |
| IRIS bf16 (incluye torre de texto y cabezas sin uso) | 3.0–3.2 GiB | medido |
| LLM Bonsai-2-9B PTQ1_0 | 2.0 GiB | archivo |
| KV del LLM, q8, 64k tokens | ~1.1 GiB | código (17 KiB/token) |
| LNCM small (22 M) | < 0.1 GiB | código |
| Buffers de cómputo del LLM | ~0.3–0.5 GiB | estimado |
| **Total** | **~7.8–8.3 GiB** | no cabe con holgura |

Por eso el router de memoria (§4) no es opcional. Hoy se convive por turnos: el entrenamiento de LNCM usa la GPU sola (5.5 GiB en este momento) y pausa por temperatura a 83 °C.

## 3. Puntos críticos de IRIS

Las rutas son de `core/crates/archie-iris-runtime/src/` en la rama `claude/laughing-lamport-5af9cs`. Todos son **exactos**: los mismos pesos dan los mismos resultados dentro de las tolerancias de `tests/parity.rs`. Van ordenados por impacto esperado.

### C1. El MoE rutea en la CPU (`model.rs`, `SparseMoe::forward`)

Por cada una de las 6 capas, el runtime hace esto:
- copia todas las probabilidades del router a la CPU con `to_vec1`;
- ordena 32 valores por token en la CPU (17 408 tokens en modo abierto);
- recorre los 32 expertos con dos copias host→GPU cada uno (`Tensor::from_vec` de ids y pesos), `index_select`, dos matmul, sesgo, GELU, escala e `index_add`.

Eso da ~65 sincronizaciones y ~300 lanzamientos por capa. La GPU se vacía en cada copia.
- La versión Python (`beta_model.SparseMoeFeedForward`) hace lo mismo sin sincronizar nada: topk en GPU, ranuras por `one_hot().cumsum()` y dos `baddbmm` sobre `[E, capacidad, D]`. Su docstring lo explica: "the step is launch-bound".
- **Arreglo**: portar ese algoritmo tal cual a candle, con un matmul por lotes en 3-D. El orden de las ranuras es el mismo que el del bucle actual (token por token, k intercalado), así que la paridad es por construcción.
- **Estimado**: el ordenamiento en la CPU solo ya cuesta ~9 ms por capa en modo abierto; con 6 capas, unos 50 ms por frame con la GPU parada.

### C2. Atención sin fusionar (`nn.rs`, `attend`)

En la GPU, cada atención hace lo siguiente:
- copia K transpuesta a memoria contigua;
- guarda los puntajes completos;
- los pasa de bf16 a f32 para el softmax y de vuelta a bf16;
- construye las máscaras con `where_cond` sobre tensores de tamaño completo.

Con 1024 tokens son 8–12 cabezas × 1024² por capa y por fila. Por eso existe `in_slices` (de 4 en 4), que además serializa el modo abierto.
- **Arreglo**: `candle-flash-attn`, que soporta sm86 en bf16 con dimensión de cabeza 64, y su variante varlen para el padding de texto.
- Sin matriz de puntajes, `in_slices` deja de hacer falta.

### C3. Todo sube a la GPU (`weights.rs:159`)

`Weights::load` hace `to_device` de los 1240 tensores, incluidas cabezas que el runtime nunca lee, y `TextTower::load` sube la torre de texto entera.
- **Arreglo**:
  - cargar en la GPU solo los prefijos que usa el modelo servido;
  - correr la torre de texto en la CPU en f32, que además queda más cerca de la referencia;
  - guardar la caché de frases (`PhraseFeatures`, ya redondeadas a f16) en disco, para que el vocabulario de 256 frases y las frases enfocadas no se recalculen nunca.
- **Estimado**: libera ~0.6 GiB de VRAM.

### C4. Bug de frases en lote (`siglip.rs:212`)

`states.narrow(1, length - 1, 1).squeeze(1)` deja una vista con stride que cuBLAS rechaza.
- **Arreglo**: `.contiguous()`. Una línea.
- Hoy la primera petición con varias frases nuevas falla en producción.

### C5. El modo abierto repite la imagen 17 veces (`detect.rs`, `detect_everything`)

`tokens.broadcast_as((17, …)).contiguous()` copia la imagen 17 veces y corre la fusión completa por fila.
- La autoatención de imagen de la **capa 0** es idéntica en las 17 filas: se calcula una vez y se reparte (exacto).
- Desde la capa 1 las filas divergen por la atención cruzada con su texto.
- El resto se ataca por planificación (§4.4): el barrido abierto solo cuando hay daño en la escena, no por reloj.

### C6. Sincronizaciones en la decodificación (`model.rs`, `decode`)

`token_logits.max().to_vec2()` lleva los puntajes a la CPU, ordena 1024 tokens por fila en la CPU, y hace una copia host→GPU por fila.
- **Arreglo**: `topk` en GPU, con el mismo desempate (índice menor primero).
- Además, `PhraseScorer` crea en cada llamada un tensor `-1e4` de tamaño completo; se cachea por forma.

### C7. Embeddings de instancia (`model.rs`, `instance_embeddings`)

Por **cada** detección baja el mapa fusionado entero (1024 × 512 en f32, 2 MiB) a la CPU y hace `roi_align` ahí. Con `embed=1` en modo abierto, hasta 60 veces por frame.
- `roi_align` es **lineal** en el mapa.
- **Arreglo**: construir en la CPU una matriz de pesos bilineales `[cajas·9, 1024]` (pequeña) y multiplicar en la GPU, una vez por fila. Mismos números.

### C8. `/act` y `/detect` codifican dos veces el mismo frame (`server.rs`, `act.rs`)

Un solo `Mutex<Service>` serializa todo, y cada endpoint vuelve a correr los dos codificadores.
- **Arreglo**: una caché de `encode_image` por digest del frame (exacto).
- Mejor todavía: `encode_fused_state`, la fusión con la frase nula, es **el mismo tensor** que necesitan:
  - las ranuras de memoria de `/act` (`fused_null` en `act.rs`);
  - la cabeza de lectura;
  - el predictor temporal;
  - el seguimiento.
- Se calcula una vez por frame y se publica como estado de escena (§5).

### C9. Preparación en el bucle caliente

Lo siguiente corre en serie con la GPU, por petición:
- decodificar el JPEG;
- `letterbox` (el bilineal exacto de Pillow);
- `to_tensor`;
- tokenizar.

**Arreglo**, la regla de aislamiento de Kimi K3: preparar en hilos de CPU el frame t+1 mientras la GPU corre el t (doble búfer), y subir con memoria fijada. **F0 mide cuánto pesa** dentro de los ~114 ms.

### C10. RoPE y parches (`vjepa.rs`)

`Rope::apply` hace `narrow` + `neg` + `cat` + `reshape` por capa, y `patchify` hace una permutación contigua.
- **Arreglo**: un kernel de RoPE fusionado, o tablas pre-intercaladas.
- Menor que C1–C3, pero son 24 copias por frame.

## 4. El router de ejecución: `archie-accel` ("megakernel" a nivel OS)

### 4.1 Qué se toma de Kimi K3 (vLLM, 2026-10-16) y qué no

| En K3 | En ArchieOS |
|---|---|
| Kernels fusionados por subproblema (23 → 13 por capa, 130 → 69 µs) | MoE agrupado (C1), flash-attention (C2), RoPE/LayerNorm/GELU con sesgo fusionados |
| Experto compartido en un stream lateral, solapado con el ruteo | Lo mismo en el MoE de IRIS: el compartido corre mientras se calculan topk y ranuras |
| Prefetch de pesos en un stream lateral durante fases sin saturar el ancho de banda | Mientras corren los codificadores (que no tocan expertos), traer de RAM los expertos que el frame anterior usó y no están en VRAM |
| Plan estático por tamaño de lote | Plan por forma: (puntos de foco × modo) ∈ {1..4} × {focused, open, act, read}; después, CUDA graphs por forma |
| Intercambio sin vallas | S-LINKs: el consumidor espera un evento CUDA en su propio stream, sin sincronizar la CPU |
| Aislamiento: preparación de entrada, muestreo y KV fuera del bucle | Decodificación, letterbox, tokenización, NMS y manejo de estado en hilos de CPU |

K3 no trata RAM ni SSD. Eso es lo nuevo aquí, porque la 3070 tiene 8 GiB para tres modelos.

### 4.2 Dónde vive cada cosa

| Dato | Lugar por defecto | Bajo presión | Por qué |
|---|---|---|---|
| Codificadores, fusión, decodificador, router + compartido | GPU | GPU | Se usan en cada frame |
| Expertos (192 × 4 MiB) | GPU | **calientes** en GPU, fríos en RAM fijada (§4.3) | 57 % de los pesos de IRIS |
| Cabeza de acción, `read_decoder`, temporal | GPU si hay sesión `/act` o lectura activa; si no, RAM | RAM | Se usan por demanda |
| Torre de texto SigLIP2 | **CPU** (f32) | CPU | Solo corre para frases nuevas |
| Caché de frases | RAM + **SSD** | SSD | Persistente entre arranques |
| LNCM de Wanda (22 M) | GPU | CPU (medido en CPU: 596 ms por respuesta completa) | Pequeño; puede caer a CPU sin morir |
| Pesos del LLM | GPU | menos capas en GPU (`-ngl`) | Se fija al arrancar `llama-server` |
| KV del LLM | GPU, q8 | slots guardados en **SSD** (`--slot-save-path`) | Cambiar de tarea restaura en vez de recalcular |
| SocialState de Wanda, memoria de escena | RAM + Vault | Vault | Ya es estado acotado |

### 4.3 Expertos: residencia, prefetch y degradación ya entrenada

1. **F0 mide el ruteo real**:
   - histograma por capa, experto y fuente (pantalla o cámara);
   - solapamiento entre frames consecutivos;
   - cuántos expertos quedan sin tokens.

   Con 1024 tokens × top-2 y capacidad 96 por experto, puede que casi todos se usen en cada frame. En ese caso la residencia parcial **no ahorra nada** a calidad completa, y solo sirve bajo presión de VRAM.
2. **Con VRAM suficiente**: los 192 expertos residentes (exacto).
3. **Bajo presión**, Policy fija un piso de calidad, y por capa quedan en GPU los K expertos más usados para la mezcla de fuentes actual.
   - Un experto no residente se trata como **capacidad 0**: sus tokens siguen solo por el experto compartido y el residual.
   - Es el mismo camino que el modelo toma cuando un experto se desborda, y que ya vio en entrenamiento (`capacity_factor` 1.5).
   - Es una degradación con fundamento, no inventada, pero **se mide** con A/B (paridad de detecciones y evaluaciones retenidas) antes de habilitarla.
4. **Prefetch**:
   - el ruteo del frame t predice el del t+1;
   - los expertos fríos que hagan falta se copian por PCIe en un stream de copia mientras corren V-JEPA y SigLIP;
   - a ~25 GB/s, 4 MiB tardan ~0.16 ms (**estimado**; F0 mide el PCIe real del host bajo WSL2).
5. **Lotes de varios puntos de foco**:
   - la capacidad del MoE se calcula sobre todo el lote, como en Python, así que mezclar imágenes cambiaría qué tokens se descartan;
   - **la capacidad se calcula por grupo (por imagen)** dentro del mismo kernel, lo que equivale a correr cada imagen sola;
   - el modo abierto mantiene la capacidad sobre sus 17 filas, como la referencia.

### 4.4 Puntos de foco: memoria contra contexto

Hay tres recursos y no compiten igual:

| Recurso | Qué consume | Crece con | Quién decide |
|---|---|---|---|
| Pasadas y expertos de IRIS | VRAM y tiempo de GPU | puntos de foco × fps × modo | `archie-accel`, con consejo de Cortex |
| Contexto del LLM | tokens de prompt: KV y un prefill **secuencial** | el texto que se le pasa | `ContextBundle` de Cortex (`token_budget`) |
| Estado (escena de IRIS, SocialState de Wanda) | tamaño fijo | no crece | actualizaciones por delta |

**Regla**: lo que se ve va a **estado**. Al contexto entra solo lo que la tarea necesita, como referencia más resumen. Así Archie puede mirar cuatro fuentes sin llenar la ventana del LLM.

Presupuesto por foco (ejemplo; los pesos salen de `FocusActivation.strength`):

| Foco | Modo IRIS | fps | Lectura | Disparo |
|---|---|---|---|---|
| Ventana activa (el dueño escribe o lee) | focused, con las frases de la tarea | máximo | sí, en las regiones dañadas | daño de pantalla |
| Segunda pantalla | barrido abierto + focused | 2–5 | si cambió el texto | daño |
| Cámara con personas | focused (personas) + embeddings | 5–10 | no | movimiento |
| Cámara quieta | nada | 0 | no | diferencia de frames |

- **Daño en lugar de reloj**: el grafo de escena ya define regiones de daño (`docs/IRIS-SCENE-GRAPH.md`: "Caret noise and unchanged frames produce no work").
  - Si una pantalla no cambió, se reutiliza el resultado anterior (exacto).
  - El barrido abierto de 517 ms ocurre cuando entra algo nuevo, no cada 2 s.
- Para leer texto pequeño de una pantalla de 1920 × 1080, sirve más un recorte de la región dañada a resolución nativa que la pantalla entera reducida a 512.
- Un canvas menor (256 = 256 tokens, ~4× más barato) es posible porque RoPE y las posiciones de SigLIP ya se interpolan. Pero el modelo se entrenó a 512: **solo con A/B**.

### 4.5 El tick y las prioridades

`archie-accel` es un proceso que posee un contexto CUDA para IRIS, el LNCM y las cabezas nuevas. `llama-server` sigue en su propio proceso. En WSL2 no hay MPS ni sched_ext, así que la prioridad entre procesos no se controla; dentro del proceso sí, con prioridades de stream.

Cada tick (~33 ms):

1. **Entradas**:
   - la cola de trabajos por foco (frames, `/act`, lecturas, preguntas a Wanda);
   - el plan de foco que aconseja Cortex;
   - el sobre que aprobó Policy (clases de prioridad, VRAM máxima por modelo);
   - el estado de memoria.
2. **Composición**:
   - los frames de todos los focos que tocan ahora van a los codificadores en **un lote**;
   - las filas de fusión de todos, en un lote con capacidad por grupo;
   - las lecturas de texto, en un lote;
   - las preguntas a Wanda, en un lote (`generate_many` ya existe).
3. **Prioridad**: voz y reflejo > IRIS en el foco activo > Wanda interactiva > barridos de fondo > LLM en lote o lecturas proactivas > entrenamiento (que hoy toma la GPU entera con el archivo de pausa; se mantiene).
4. **Memoria**: prefetch y desalojo de expertos y cabezas, y el modo del LLM (`cpu`, `gpu`, `gpu-max` de `wanda-llm.sh`) según la VRAM libre.

## 5. S-LINKs

### 5.1 Qué es

Un S-LINK es un canal **tipado, acotado y versionado** de un productor a uno o varios consumidores. Lleva el último estado más un anillo de deltas.

- **Publicar**: `publish(rev, payload)`. Nunca bloquea al productor.
- **Leer**: `latest()` devuelve la última revisión completa, con seqlock y sin esperar. `subscribe(after_rev)` entrega deltas.
  - Si el anillo se desborda, el consumidor recibe un "hueco" y relee el estado. Como el contrato es de estado, perder deltas no corrompe nada.
- **Dentro de `archie-accel`**: el tensor queda en la GPU con un evento CUDA registrado al publicar, y el consumidor espera ese evento en su stream. Sin copias y sin sincronizar la CPU.
- **Entre procesos** (`archie-wanda`, harness, `llama-server`):
  - socket Unix para control;
  - memfd compartido con seqlock para la carga;
  - eventfd para despertar.
- **Contrato**, con la misma disciplina que `ContextEvent` del feed IoT:
  - esquema con `deny_unknown_fields`;
  - tamaños máximos;
  - digest de contenido;
  - `observed_at` de la fuente;
  - grado de sensibilidad. El texto de pantalla es `Restricted` y nunca sale a la nube.
- **Un S-LINK lleva datos, nunca autoridad.**

### 5.2 Los enlaces

| Enlace | Productor → consumidor | Carga | Cuándo publica |
|---|---|---|---|
| `scene.state` | IRIS → acción, lectura, temporal, Cortex | ranuras de escena (8 × 512) + pistas con id estable | cada frame con daño |
| `scene.text` | IRIS (lectura) → Wanda LNCM | líneas nuevas o cambiadas, con id de pista | por tick, en lote |
| `scene.people` | IRIS → Wanda | embeddings de instancia de personas, para cruzar con su roster | al aparecer o cambiar |
| `social.intent` | Wanda → LLM, harness | decisión rápida + **nivel grueso** del plan (VQ nivel 0) | al terminar la primera pasada |
| `social.focus` | Wanda → Cortex | quién o qué importa ahora (sugerencia de `FocusActivation`) | al cambiar |
| `llm.partial` | LLM → harness, joiner | tokens en streaming | continuo |
| `task.intent` | harness → joiner | herramienta o archivo que la respuesta parcial pide, como **propuesta** | cuando se detecta |
| `focus.plan` | Cortex → `archie-accel` | consejo de presupuesto por foco (§6) | al cambiar el foco |

### 5.3 Procesamiento en paralelo y post-procesado en tanda

**Hoy, en serie**: LLM, después LNCM. Con los números de CPU medidos: el LNCM agrega 596 ms después del LLM.

**Con S-LINKs**:

1. `gate` + `quick` de Wanda (165 ms medido en CPU) corren **a la vez** que el prefill del LLM.
2. La intención social sale por `social.intent` antes de que el LLM empiece a escribir, y se le inyecta como sufijo corto del prompt.
3. `compose` (393 ms medido en CPU; en GPU, a medir) corre en paralelo con la decodificación del LLM.
4. **Join**: cuando termina el productor principal, el joiner toma la **última** revisión de cada enlace, sin esperar a nadie, y hace en una tanda:
   - `guard_names` con el roster de Wanda;
   - MBR entre candidatos;
   - chequeo de Policy;
   - formateo (voz o notificación).
5. La latencia del turno pasa de la **suma** a ~**máximo**(LLM, LNCM, tick de IRIS) + post-procesado.

**Lo honesto**:
- El que manda la latencia es el LLM: 405 s para un prompt de 2k tokens en CPU (**medido**). Lo grande es llevarlo a la GPU con caché de prompt y no llenarle el contexto. Los S-LINKs le sacan la parte serial a todo lo demás.
- `llama-server` no acepta contexto nuevo a mitad de una generación. "Inyectar" significa cancelar y reenviar con el prompt extendido: con `--cache-reuse` solo se recalculan los tokens agregados.

### 5.4 El harness: trabajo cognitivo con la autoridad intacta

El router conecta a quien entiende con quien hace:
- Wanda decide qué corresponde socialmente, por ejemplo "responderle a X en tono cálido, confirmar el viernes".
- El router arma el trabajo para el LLM o las herramientas: redactar, agendar, programar.
- El harness parsea `llm.partial` **mientras llega** y prepara lo que pueda sin efectos: abrir el archivo, traer contexto, compilar en un sandbox.

**Ejecutar** sigue siendo Cerebra → Policy → lease → Stage. Un `task.intent` es una propuesta y no autoriza nada. Así el OS entiende el contexto social de Wanda en tiempo real aunque Wanda no sepa programar: aporta el *qué* y el *para quién*, y el LLM y el harness aportan el *cómo*.

## 6. Cortex: qué agrega y qué no puede hacer

**Lo que ya existe** (`cortex/include/archie/cortex/`):
- `FocusState` con activaciones (fuerza, vida media, fijada);
- `ContextRequest.token_budget` y sensibilidades permitidas;
- `NeuronRegistry` con `CapabilityDescriptor` (entradas, salidas, campos de telemetría);
- `AnalyticalHead::Resource`;
- `TrainingEvent` con `UtilityObservation` (latencia, memoria, energía, éxito);
- `predict_process`;
- `ProposalKind::CandidateSchedulerChange`;
- `ModelBackend::TensorRt`.

**Lo que se agrega**, todo consultivo:

1. **Puntos de foco como nodos**: pantallas como `Device` y cámaras como `Sensor`. Su activación sube con:
   - la actividad del dueño;
   - la importancia social que manda Wanda (`social.focus`);
   - la tarea en curso.
2. **`ResourceAdvice`** (la carga de un `CandidateSchedulerChange`): por foco, la parte sugerida del cómputo de IRIS, el modo, las frases y si lee texto. Por modelo, la latencia y la memoria que predice `predict_process`. Para el LLM, cómo repartir entre tokens de contexto y resúmenes de estado.
3. **Descriptores de capacidad** para IRIS, la lectura, el LNCM de Wanda y el LLM, con sus campos de telemetría.
4. **Ciclo de aprendizaje**: `archie-accel` emite un `TrainingEvent` por trabajo (latencia, VRAM, energía). Wanda y el harness emiten `ContextUseful` o `MissingContext`. La cabeza `Resource` aprende qué reparto de foco dio contexto útil.

**Lo que no hace**:
- no toca la GPU;
- no planifica;
- no concede nada.

Su propio contrato lo dice: `predict_process` "no puede cambiar política de scheduler, prioridad ni grants". `archie-accel` aplica el consejo dentro del sobre de Policy, y cambiar el sobre es una propuesta que Policy decide.

## 7. Puente OCR de IRIS → primera capa del LNCM de Wanda

### 7.1 Hoy

- `docs/IRIS-OCR.md` dice que no hay motor de OCR incluido, y que el texto nativo (DOM, accesibilidad) evita el OCR.
- La release **sí trae** una cabeza de lectura, `read_decoder` (`beta_model.ReadDecoder`, `read_head: true`), que el runtime Rust no porta:
  - decodificador autorregresivo de caracteres, 2 capas, ancho 512, hasta 32 caracteres;
  - su alfabeto incluye áéíóúñ¿¡;
  - lee desde `encode_fused_state`, la escena fusionada con la frase nula, que es el mismo tensor que C8 propone publicar.
- El `generate` de Python recalcula todo el decodificador en cada paso, sin caché de KV.

### 7.2 Etapa A, sin reentrenar nada

1. **Orden de fuentes de texto**:
   1. accesibilidad o DOM: exacto y gratis;
   2. la lectura de IRIS sobre las regiones dañadas sin texto estructurado;
   3. un especialista OCR externo, que hoy no existe.
2. **Portar `read_decoder` a Rust** con caché de KV:
   - la K y V de la memoria (1024 × 512) se proyectan una vez por recorte y capa;
   - las autoatenciones crecen paso a paso;
   - todas las regiones de texto del tick van en un lote.
3. **Caché de líneas** por (id de pista, digest de los píxeles de la región): si no cambió, no se relee.
4. **Delta a Wanda**: las líneas nuevas o cambiadas salen como `SceneTextDelta {pista, texto, fuente, confianza, observed_at}` por `scene.text`. Cada tick, o cada N líneas, van todas juntas a una llamada `update` del lector del LNCM como `chunk`. El `SocialState` se actualiza en su lugar.
5. Esto ya cumple "directo a la primera capa": los tokens del chunk entran por el embedding del LNCM (`vq.embed_tokens` → `tok_proj` + `pos`, en `generator.rs`, `embed_prompt`). El costo es pasar por texto y re-tokenizar.

### 7.3 Etapa B, entrenando solo la cabeza

Reentrenar la cabeza de lectura con el tronco congelado para que emita **ids del códec de Wanda** (su BPE) en lugar de caracteres:
- los ids van por el S-LINK y entran **sin conversión** a `embed_prompt` del LNCM;
- una línea de 32 caracteres son ~8–10 tokens BPE (**estimado**), así que hay 3–4 veces menos pasos;
- después se pasa a lectura NCM, grueso → fino por niveles en paralelo, en lugar de token a token.

### 7.4 Etapa C: LNCM v2 con segmento de escena

El prompt del LNCM es bidireccional: `forward_kv` no lleva máscara (**código**). Cualquier cambio en el prompt invalida toda su caché de KV.

La v2 agrega un segmento `<scene>` cuyas posiciones son vectores de IRIS proyectados:
- se atiende con una máscara por bloques, para que su KV se calcule y se **reemplace sola** cuando cambia la escena;
- el resto del prompt queda en caché, y así se actualiza solo lo necesario, a nivel de KV;
- va junto con RoPE y un `prompt_len` de 2048–4096, que ya estaba en el plan de Wanda.

Requiere una corrida de entrenamiento propia.

### 7.5 Estado en vez de compactación

Nada se resume ni se recorta para que entre:
- la escena es estado fijo con pistas;
- el texto visto actualiza el `SocialState`;
- el LLM recibe referencias y resúmenes por el `ContextBundle` de Cortex.

La conversación puede crecer sin límite porque el lector del LNCM ya trabaja por chunks con estado persistente.

## 8. LNCM en lugar de LoRA, NCM en lugar de difusión

**Hoy**:
- `lora.py`: skills como LoRA de rango 8 sobre la cabeza de acción y la última capa de fusión, una activa a la vez;
- el runtime Rust las rechaza (`/act?adapter=`).

**Propuesta: cabeza IRIS-LNCM de acción**, con el tronco congelado y el mismo SHA-256:

| | LoRA (antes) | LNCM (ahora) |
|---|---|---|
| Qué es una skill | deltas de pesos en ~20 capas | **vectores de skill** dentro del prompt (estado persistente, como el `SocialState` de Wanda) |
| Cambiar de skill | cambiar pesos | cambiar unos KB de vectores |
| Dos sesiones con skills distintas en el mismo lote | no, sin kernels multi-LoRA | **sí**: son filas distintas con los mismos pesos |
| Aprender una skill en REM | backprop en los adaptadores | optimizar los vectores de skill, o que el lector los actualice desde demostraciones |
| Salida | 8 controles por llamada (clasificación) | plan de 64 controles por niveles VQ (4/16/64), en 3 pasadas |
| Kernels | propios | los mismos de Wanda (`archie-nn` común), optimizados una vez |

- El prompt de la cabeza:
  - la memoria T1–T4 que IRIS ya construye;
  - las ranuras de escena;
  - la instrucción;
  - los vectores de la skill.
- La cabeza actual (`ActionMemoryHead`) sigue como campeona. La nueva se promueve solo si iguala o supera en las evaluaciones retenidas (puntero GUI, composite v3).
- IRIS no tiene difusión (**código**: ni en el runtime ni en Foundry). NCM entra donde hoy hay generación paso a paso:
  - acciones (arriba);
  - lectura (§7.3).

## 9. Contexto del LLM sin morir

- Bonsai-2-9B es híbrido: 8 de 32 capas llevan KV y el resto usa DeltaNet con estado fijo. Cuesta 17 KiB por token en q8 (**código**) y se entrenó hasta 262 144 tokens.
- **Prefijo estable primero**, para que `--cache-reuse` acierte, en este orden:
  1. sistema;
  2. resumen del estado durable;
  3. contexto de la tarea (el `ContextBundle`);
  4. turno.
- **Slots en SSD** (`--slot-save-path`): guardar y restaurar el KV por conversación o tarea, en vez de recalcular el prefill. **Hay que verificar** que el guardado de estado de llama.cpp incluya el estado recurrente de las capas DeltaNet.
- Nada de OCR crudo ni transcripciones en el contexto: estado más referencias.
- Los slots paralelos de `llama-server` coinciden con `parallel_queries` de Wanda (ya implementado en `perf/wanda-runtime`).

## 10. Fases, verificación y metas

Las metas son objetivos a verificar, no promesas. Ninguna fase cambia el tronco de IRIS ni sus pesos.

| Fase | Qué | Cómo se verifica | Meta |
|---|---|---|---|
| **F0** | Perfil con nsys de focused y open; histogramas de ruteo por fuente; VRAM por componente; tiempo de host (JPEG, letterbox); PCIe real | informe con números | saber dónde están los ~114 ms |
| **F1** | Exactos: C4 (bug), C1 (MoE en GPU), C3 (pesos usados, torre de texto en CPU, caché de frases en SSD), C5 (capa 0), C6, C7, C8 (caché de frame y estado de escena), C9 (pipeline) | `tests/parity.rs` con las mismas tolerancias; detecciones idénticas; bench en la 3070 | open ≤ 300 ms, focused ≤ 90 ms, −0.6 GiB de VRAM |
| **F2** | Kernels: flash-attention sm86, RoPE/LayerNorm/GELU fusionados; opcional, TensorRT para los dos codificadores (formas fijas de 512) como backend detrás de la misma prueba de paridad | paridad + bench | focused ≤ 50 ms (20 fps), open ≤ 200 ms |
| **F3** | `archie-accel`: dueño único de la GPU para los modelos candle, lotes multi-foco con capacidad por grupo, prioridades por stream, residencia y prefetch de expertos, presupuesto de VRAM con `llama-server`, disparo por daño | bench con 1–4 focos; A/B de calidad si hay expertos no residentes | ≥ 15 fps por foco activo cuando cambia; ~0 de GPU si nada cambia |
| **F4** | S-LINKs, puente OCR etapa A, port de `read_decoder` con caché de KV, join en tanda, `ResourceAdvice` y telemetría en Cortex | pruebas de contrato (límites, versiones, huecos, sensibilidad); latencia por turno | turno ≈ máx(modelos) + post |
| **F5** | Entrenamiento con el tronco congelado: cabeza IRIS-LNCM de acción (skills como estado), lectura con el códec de Wanda, LNCM v2 con segmento de escena | A/B en evaluaciones retenidas; promover solo si iguala o mejora | sin regresión |
| **F6** | Megakernel persistente por modelo solo donde F0–F3 muestren límite por lanzamientos: LNCM small y cabezas pequeñas a lote 1. Los codificadores de IRIS están limitados por cómputo: los atienden mejor flash-attention o TensorRT | bench | — |

Para el SO, sigue valiendo §3 del plan anterior: cgroups por servicio, systemd y `oom_score_adj`. El OOM que mató a IRIS hoy no se repite.

## 11. Riesgos y lo que todavía no sé

- **Cómo se reparten los ~114 ms** del modo enfocado entre preparación y codificadores. F0 lo dice; si domina la preparación, C9 vale más que los kernels.
- **Si el ruteo de expertos es casi uniforme**: entonces la residencia parcial solo sirve bajo presión, con pérdida que hay que medir.
- **CUDA graphs con candle**: el allocator por operación puede impedir la captura. Hay que investigarlo; TensorRT es la alternativa probada para los codificadores.
- **La estabilidad de la GPU** bajo carga combinada: la caída del 2026-10-16 no tiene causa determinada, y el entrenamiento pausa por temperatura a 83 °C. Hacen falta límite de potencia y guardián térmico antes de cargar todo junto.
- **La calidad de `read_decoder`**: no hay evaluación propia en el runtime. Es un lector de 32 caracteres por recorte, no de páginas enteras.
- **El estado recurrente del LLM híbrido** en el guardado de slots (§9).
- **WSL2**: sin MPS ni sched_ext, la prioridad entre `llama-server` y `archie-accel` la decide el driver. En la ISO nativa sí habrá control.

## 12. Por dónde empezar

F0 y F1 en la rama de IRIS (`archie-iris-runtime`), en este orden:
1. **C4**: una línea, y hoy rompe producción.
2. **C1**: el MoE en GPU, con paridad por construcción.
3. **C3 y C8**: VRAM y doble codificación.
4. El perfil de F0 con todo eso puesto.

La GPU está ocupada por el entrenamiento del LNCM (tokenizador, después el generador de 60 000 pasos). Los cambios de F1 se desarrollan y se prueban en la CPU con `tests/parity.rs`, y se miden en la 3070 cuando el entrenamiento la libere.
