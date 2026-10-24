# Implementación del plan IRIS / archie-accel / S-LINKs / Cortex — 2026-10-21

Registro de lo implementado sobre `_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md` (rama
`perf/wanda-runtime`) y el paso 3 de `HANDOFF-WANDA-IRIS-2026-10-20.md`. Rama
`dev/gracious-franklin-hu2brn`, PR https://github.com/PentaVM/ArchieOS-Nvidia/pull/47 (apilado
sobre el #35). Commits como `PentaVM <n18867428@gmail.com>`.

Cada punto dice cómo se verificó. **Nada se corrió en una GPU:** el contenedor donde se
escribió no tiene GPU. Lo que depende de la 3070 está en §6 con el comando que lo mide.

## 1. Puntos críticos de IRIS (C1–C10)

| | Qué se hizo | Dónde | Verificación |
|---|---|---|---|
| C1 | MoE sin lecturas al host en GPU (top-k por máximos, colas por conteos acumulados por bloques, productos por lotes); la CPU conserva la ruta del host, más rápida ahí (539 vs 722 ms enfocado, 4.0 vs 6.3 s abierto, medido en el contenedor) | `model.rs` `SparseMoe` | Las dos rutas idénticas bit a bit en CPU f32; arreglo de sesgos `[E,1,H]` del dueño (`9fc97b9`) incluido |
| C2 | flash-attention v2 (feature `flash-attn`) para las atenciones sin máscara; sin `in_slices` cuando está activa | `nn.rs` `attend` | Compila (§6); no corrió |
| C3 | Pesos leídos del zip a demanda: solo lo usado llega a la GPU; torre de texto en CPU f32 por defecto (`--text-device`); solo `text_model.*` sube al dispositivo; caché de frases en disco (`--phrase-cache`, archivos 0600 por digest); vocabulario precalentado | `weights.rs`, `phrases.rs`, `siglip.rs`, serve | `tests/weights.rs` sobre un `.pt` de forma real sin PyTorch; **arreglado un bug de offset** previo (el offset de candle ya está en bytes y se multiplicaba otra vez) |
| C4 | `.contiguous()` del estado pooled (frases en lote en CUDA) | `siglip.rs` | Revisión del dueño; no reproducido en GPU |
| C5 | Autoatención de la capa 0 una vez para todas las filas de la misma imagen (modo abierto y `/act`) | `fuse_shared_image` | Bit a bit igual a repetir la imagen |
| C6 | Selección de consultas en el dispositivo con el mismo desempate que el sort estable (claves f64 distintas) | `top_tokens_on_device` | Igual al host con empates, ceros, negativos y filas bloqueadas |
| C7 | `roi_align` como matriz bilineal × mapa en GPU; embeddings del modo abierto en lote por chunk | `roi_align_matrix`, `instance_embeddings` | Igual a `roi_align` a redondeo |
| C8 | Caché de la codificación por digest del canvas, compartida entre `/act` y `/detect` (exacta: los codificadores no llevan LoRA) | `encode_frame` | Digest probado; ruta probada en `tests/pipeline.rs` |
| C9 | Decodificación JPEG y letterbox fuera del lock del modelo; canvas subido en u8 y convertido en el dispositivo | `server.rs` `with_frame`, `canvas_tensor` | Conversión bit a bit igual al host |
| C10 | RoPE en una pasada (CPU) o con una permutación con signo (GPU) | `vjepa.rs` | Las dos rutas bit a bit iguales a la anterior |

No hecho de §3: el experto compartido en un stream lateral y CUDA graphs (candle 0.9 no
expone streams ni captura de grafos); memoria fijada para la subida (candle no la expone).

## 2. archie-accel (F3, §4)

Crate nuevo `core/crates/archie-accel`, sin librería de tensores:

- `damage`: daño por tiles con umbral de ruido, regiones conectadas, regla del cursor
  (`docs/IRIS-SCENE-GRAPH.md`), y daño desde los rectángulos del compositor.
- `track`: seguimiento por block matching (medias removidas, plantilla del keyframe, posición
  predicha por velocidad, todas las posiciones enteras: sin alias en texturas finas). **20
  cajas en 640×480: 3.9 ms por cuadro con 4 hilos del contenedor** (bench `--ignored`).
- `plan`: reuse / track / infer por foco, barrido abierto solo con escena nueva y no más seguido
  que lo configurado, clases de prioridad (voz y reflejo > foco activo > Wanda > fondo > lote >
  entrenamiento), modelo de costos aprendido de mediciones, admisión por tick.
- `memory`: libro de VRAM que rechaza pasarse del sobre; residencia de expertos exacta si entra
  todo, parcial solo si el sobre lo permite y sobre un piso de calidad.
- `advice`: `ResourceAdvice` de Cortex (`deny_unknown_fields`) recortado por el sobre de Policy:
  baja, nunca sube prioridad, tasa ni memoria.

En IRIS (`archie-iris-runtime`):

- `/live` responde cada cuadro sin esperar al modelo (daño + seguimiento en CPU); pide lecturas
  a una cola donde gana el cuadro más nuevo por foco; un worker las corre y re-ancla. Cada caja
  dice `inferred`, `reused`, `tracked` o `lost` y conserva su id entre inferencias.
  `/live/advice` recibe el consejo de Cortex; `GET /live` informa focos y costos. Sobre:
  `--live-max-fps`, `--live-max-priority`. Modo en vivo en la página de demo.
- `detect_many`: hasta 4 focos en un pase con capacidad del MoE por imagen (`fuse_per_image`):
  cada foco da lo mismo que solo (bit a bit en la ruta del host; `tests/pipeline.rs` lo compara
  en el modelo completo).
- Residencia de expertos (`set_expert_residency`): los ausentes salen de la GPU y actúan con
  capacidad 0. Apagada por defecto: cambia resultados y requiere A/B.
- `--record-expert-usage` + `GET /experts`: histogramas de ruteo por capa (F0).

**60 fps.** La meta del dueño es 60 fps. Por las cuentas del plan (§2.3), un cuadro enfocado son
≈0.54 TFLOP: a 40 TFLOPS pico serían 13.5 ms al 100 % de eficiencia, ~27 ms con una eficiencia
realista. La inferencia completa por cuadro no llega a 60 fps en la 3070. Lo que esta rama hace
es responder **cada** cuadro a la tasa de la cámara (seguimiento y reutilización, ~4 ms en CPU)
y leer con el modelo tan seguido como dé la GPU. El bench mide las dos cosas por separado
(`frames_answered_per_second` y `model_reads_per_second`). Si la escena no cambia, el costo es ~0.

No hecho de §4: prefetch de expertos por PCIe en un stream de copia y prioridades por stream
(candle no expone streams); el presupuesto negociado con `llama-server` en vivo (el ledger
existe; falta conectarlo a `wanda-llm.sh`); canvas de 256 (requiere A/B, el modelo se entrenó a
512).

## 3. S-LINKs (F4, §5)

Crate nuevo `core/crates/archie-slink`:

- En proceso: último mensaje + anillo; un lector atrasado recibe un hueco y el estado.
- Entre procesos (Linux): memfd con tamaño sellado, dos slots con seqlock, eventfd, descriptores
  por socket Unix 0600 (`Exporter`, `connect`). Un lector a máxima velocidad contra un escritor
  sin pausa (20 000 mensajes) no vio ningún mensaje roto en 8 corridas.
- Mensajes con link, revisión, `observed_at`, `PrivacyGrade` (de `archie-types`) y SHA-256;
  `deny_unknown_fields`, tamaños máximos, números finitos. Contenido de pantalla, cámara,
  conversación y personas: `Restricted` (no sale de la máquina). `task.intent` es propuesta.
- Los 8 enlaces de §5.2 tipados; joiner que toma la última revisión de cada uno y corre el
  post-procesado en una tanda (§5.3).
- IRIS publica `scene.state` en cada cuadro en vivo y lo exporta con `--slink-dir`.

No hecho: el evento CUDA para pasar tensores entre modelos sin sincronizar la CPU (candle no
expone eventos); el lado de Wanda (ver §7).

## 4. Cortex (§6)

`cortex/include/archie/cortex/resources.hpp`: `ResourceAdvice` como carga de
`CandidateSchedulerChange` (reparto por activación con vida media, modo por tipo de fuente,
predicciones de `predict_process`), validación, `focus_plan_json` con el esquema que acepta
`archie-accel` (fixture compartido `protocols/fixtures/focus-plan.json`, probado byte a byte en
C++ y leído en Rust), descriptores de capacidad para `iris.detect`, `iris.read`, `wanda.lncm` y
el LLM con frontera de autoridad "solo propone". Cortex sigue sin tocar la GPU, planificar ni
conceder.

No hecho: la IPC que lleva `TrainingEvent` de `archie-accel` a Cortex (Cortex no los recibe
por IPC hoy).

## 5. Puente OCR, LNCM por LoRA, contexto del LLM (§7–§9)

- §7.2 etapa A: `archie-slink::text` (qué regiones leer, fuente preferida por región, solo
  líneas nuevas, lotes al `update` de Wanda por tamaño o edad; trait `StateUpdate`).
  **`read_decoder` no está portado**: hace falta el código de `archie_iris_foundry`
  (`beta_model.ReadDecoder`), que no está en el repo. Hasta entonces las fuentes son
  accesibilidad/DOM.
- §7.3–7.4, §8 (lectura con ids del códec de Wanda, LNCM v2 con segmento de escena, cabeza
  IRIS-LNCM de acción): requieren entrenamiento. No hechos.
- §9: `archie-model-runtime::llama_slots`: prompt con prefijo estable y nombre de slot por su
  digest; guardar, restaurar y borrar slots de un `llama-server` local (solo loopback, nombres
  sin separadores). **Falta verificar** que el archivo de slot de llama.cpp incluya el estado
  recurrente de las capas DeltaNet (comparar `n_restored`). `wanda-llm.sh` necesita
  `--slot-save-path` (script del dueño en `perf/wanda-runtime`, no tocado).

## 6. Qué medir en la 3070 y cómo

`scripts/iris/verify-rtx3070.sh` (en la WSL; se niega si el LNCM está entrenando, salvo
`--while-training`):

1. pruebas de CPU (release);
2. build CUDA 12.8 con `CARGO_BUILD_JOBS=3` (y `--flash` para flash-attention);
3. con `--reference`: paridad contra Python en CPU y **en GPU f32** (`IRIS_DEVICE=cuda`): es la
   prueba de las rutas de dispositivo de C1, C6 y C7;
4. `archie-iris-bench` en bf16 con VRAM muestreada: enfocado, abierto, 4 focos, y una cámara a
   60 fps por el pipeline en vivo. Escribe `_dev/verification/rtx3070/iris-f1-f4-*.json` con
   las metas (F1: enfocado ≤ 90 ms, abierto ≤ 300 ms; F2: enfocado ≤ 50 ms; 60 cuadros
   respondidos por segundo con p90 ≤ 16.7 ms) marcadas cumplidas o no.

Compilación verificada en el contenedor: `--features cuda` con nvcc 12.8 y
`CUDA_COMPUTE_CAP=86`, clippy limpio (Rust 1.88 y 1.97). `--features flash-attn`: ver §8.

## 6b. Medido en la 3070 (2026-10-21, `_dev/verification/rtx3070/iris-f1-f4-20261021.json`)

bf16, sin flash-attention, tarjeta limitada a 132 W (de 270 W), entrenamiento en pausa, cuadro
de 1280×720:

| | Medido | Meta |
|---|---:|---:|
| Cuadros respondidos en vivo (cámara a 60 fps) | 60/s, p90 2.5 ms | 60/s: cumplida |
| Lecturas del modelo en vivo | 6.5/s | — |
| Enfocado | 132.5 ms | F1 ≤ 90: no |
| Enfocado sobre un cuadro ya codificado | 56.2 ms | — |
| Encoders | 76.1 ms | — |
| Abierto (17 filas) | 689.5 ms | F1 ≤ 300: no (con el límite de potencia) |
| 4 focos en un pase | 454 ms | — |
| VRAM total (con 0.9 GiB del escritorio) | 3.1 GiB | — |

Lo que sigue para bajar el enfoque: correr el bench con `IRIS_PROFILE=1` (tiempos por etapa,
`4429b6f`) para ver dónde están los 56 ms de fusión y decodificación de una fila, y medir con
`--flash`. Después del cambio `be8ca68`, el camino que responde cada cuadro ya no arma el canvas
del modelo (18 ms en el contenedor para 1280×720; ahora 3.2 ms por cuadro con 20 cajas).

`read_decoder` quedó portado con caché de KV (`e2394b0`, `POST /read`), validado contra Python
con los pesos reales: es la fuente `IrisRead` del puente de texto.

## 7. Integración con Wanda

El LNCM de Wanda vive en `perf/wanda-runtime` (sobre `codex/wanda-context`, PR #38), fuera de
esta rama. Un merge de prueba con la rama de IRIS choca en `archie-voice` (`Cargo.toml`,
`src/bin/archie-voice.rs`, `src/nemotron.rs`: el veil de privacidad del #35 contra la voz de
Wanda) y en `Cargo.lock`. Es una fusión semántica en un límite de privacidad; no se resolvió a
ciegas. Para conectar Wanda:

1. abrir el PR de `perf/wanda-runtime` y resolver esos conflictos con el veil del #35;
2. en `archie-wanda`: implementar `archie_slink::text::StateUpdate` sobre el `update` del lector
   del LNCM, conectarse a `scene.state.sock`, publicar `social.intent` al terminar `quick` y
   `social.focus` al cambiar;
3. en el cliente LLM de Wanda: `PromptLayout` + `SlotStore`.

## 8. Pendiente, en orden

1. **El entrenamiento del LNCM** (generador de 60 000 pasos en la 3070) y lo que sigue según el
   handoff: calibrar, evaluar, pasar el LLM a GPU. No se puede hacer desde la nube.
2. Perfilar por etapas en la 3070 (`IRIS_PROFILE=1`) y probar `--flash` para bajar el enfoque a la meta de F1.
3. ~~Portar `read_decoder`~~: hecho (`e2394b0`).
4. Integración con Wanda (§7).
5. F5 (entrenamientos con el tronco congelado) y F6 (megakernel persistente donde F0 muestre
   límite por lanzamientos).
