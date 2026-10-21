# Handoff: Wanda (LNCM + LLM), IRIS y el plan de Cortex — estado al 2026-10-20 23:22 (-03)

Archivo maestro para continuar con otra IA. Todo lo de abajo fue verificado en la máquina,
salvo lo marcado "pendiente" o "estimado".

## 0. Quién es quién, reglas del dueño

- **Dueño:** PentaVM, GitHub `PentaVM`, dueño de `PentaVM/ArchieOS-Nvidia`. Habla español de Argentina.
- **Commits siempre como `PentaVM <n18867428@gmail.com>`**:
  - La config local de git todavía dice `PentaWS`, que es una cuenta vieja. No hay que usarla.
  - Commitear con `git -c user.name=PentaVM -c user.email=n18867428@gmail.com commit …`.
  - Antes de pushear, verificar con `git log --format='%an <%ae>' -1`.
  - El trailer actual es `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Esfera / GUI:** diseño de Xavi (`ByXavi270`, GitHub `XaviCottier`), implementación de PentaVM. `AGENTS.md` ya da crédito a los dos.
- **Decisiones del dueño, vigentes:**
  - Nunca difusión: donde haga falta generación iterativa se usa **NCM**.
  - Las skills de IRIS **no** van con LoRA, van con **LNCM**.
  - "Megakernel" significa **orquestación a nivel OS**: ruteo de expertos, estado SSD/RAM/CPU/GPU, presupuesto por punto de foco (pantallas, cámaras). Al estilo de https://vllm.ai/blog/2026-10-16-kimi-k3-megakernel.
  - **S-LINKs**: enlaces de software entre IAs que pasan contexto a mitad del procesamiento, con post-procesado en tanda.
  - El experto OCR de IRIS se rutea a la primera capa del LNCM de Wanda, con actualización de estado en vez de compactación.
  - Presupuesto Nebius aprobado: **$22** para destilación. Ya se usó en el pipeline de datos; ver `~/wanda-lncm/data/ledger.jsonl`.

## 1. Máquina y cómo operar

**Hardware y sistema:**
- Windows 10 con una RTX 3070 de 8 GB (driver 572.16, CUDA máx. 12.8).
- WSL2: distro **`ArchieOS-Operator-RTX3070-Test`** (Arch), usuario `archie`, **7.7 GiB de RAM**.
  - No tiene MPS ni sched_ext.
  - Para correr algo: `wsl -d ArchieOS-Operator-RTX3070-Test -u archie -- bash <script>`.

**Reglas aprendidas a la fuerza:**
- **Nunca correr trabajos largos en pestañas de terminal**: al cerrar la pestaña muere el entrenamiento. Usar `setsid -f`; ya lo hace `wanda-train-start.sh`.
- `pkill -f` / `pgrep -f` matchean su propia línea `bash -c`. Usar el patrón con corchetes: `pgrep -f '[t]raining/…'`.
- Desde PowerShell, `$` y las comillas se rompen dentro de `wsl bash -c`: escribir scripts en archivos.
- Desde Git Bash, anteponer `MSYS_NO_PATHCONV=1` para que no convierta `/mnt/c`.
- **Builds con `CARGO_BUILD_JOBS=3`**: una compilación sin límite mató a IRIS por OOM (WSL tiene poca RAM). Ya está en `wanda-train-cuda128.sh`.
- CUDA en WSL:
  - Usar el toolkit conda **12.8** en `/home/archie/iris/cuda128`, no el `cuda` 13 de Arch (`CUBLAS_STATUS_NOT_INITIALIZED`).
  - Las raíces CUDA van en `targets/x86_64-linux`.
  - `PATH` debe incluir `/usr/lib/wsl/lib`, que tiene `nvidia-smi` y `libcuda`.
  - `NVCC_CCBIN` apunta al g++ de conda y `CUDA_COMPUTE_CAP=86`.
- Hay un trabajo de **otra sesión** (Codex) que comparte la GPU: el worker Bonsai2 (`archie_wanda.cli worker`, supervisado por `ensure-worker.sh` hasta el 2026-10-23 17:00).
  - Se le pide pausa con el archivo `/home/archie/wanda/run/hold`; `train-local.sh` lo hace solo.
  - No hay que tocarlo.

**Scripts de operación** (`C:\Users\Penta\Desktop\ArchieOS\`, fuera del repo):

| Script | Qué hace |
|---|---|
| `wanda-train-start.sh` | arranca o reanuda el entrenamiento LNCM desacoplado (`setsid`) y el watcher del LLM |
| `wanda-train-cuda128.sh` | entorno CUDA 12.8 + `STAGES=train WANDA_PRESET=small` + `CARGO_BUILD_JOBS=3`, luego `train-local.sh` |
| `wanda-llm.sh cpu\|gpu\|gpu-max` | `llama-server` de Wanda en 127.0.0.1:8081; ver la tabla de modos abajo |
| `wanda-llm-after-training.sh` | espera el fin del entrenamiento y mueve el LLM a la GPU **solo si el log dice `done: checkpoint`**; si hubo crash, no lo mueve |
| `wanda-monitor.bat` / `.ps1` | monitor del dueño |
| `PR-esfera.md` | texto del PR de la esfera, que el dueño crea a mano |

Modos de `wanda-llm.sh`:

| Modo | Contexto | Detalle |
|---|---|---|
| `cpu` | 32k | 12 hilos, 1 slot |
| `gpu` | 64k por slot | 4 slots |
| `gpu-max` | 128k | — |

**Revisar el entrenamiento:**
```bash
tail -n 5 ~/wanda-lncm/logs/run.out                 # etapas y errores
tail -n 3 ~/wanda-lncm/ckpt-small/train-log.jsonl   # pasos, loss, pausas térmicas
nvidia-smi --query-gpu=memory.used,temperature.gpu --format=csv,noheader
```
Para frenar limpio: `touch ~/wanda-lncm/ckpt-small/STOP`. Para reanudar: `bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-train-start.sh`.

## 2. Estado ahora mismo (23:22)

| Cosa | Estado |
|---|---|
| Entrenamiento LNCM (preset small, 22.1 M parámetros) | **relanzado a las 23:20**. Compiló, pidió la pausa al worker Bonsai2 (23:22:13) y arranca el **generador: 60 000 pasos, 4 micro-lotes de 2 filas** (lote efectivo 8) en `cuda:0` |
| Tokenizador (VQ) | **terminado**: 20 000/20 000, precisión 0.9995, token_accuracy 0.9997, exact_sequences 0.95. Guardado en `~/wanda-lncm/ckpt-small/vq.safetensors` |
| Generador, primer intento (22:50) | **falló por OOM** en el paso 0: lote 8 no entra en 8 GB. Corregido (§3.3) |
| LLM de Wanda (Bonsai-2-9B PTQ1_0) | en **CPU** (`wanda-llm.sh cpu`). Pasa a la GPU cuando el entrenamiento termine bien |
| IRIS | **caído** desde un OOM (lo mató una compilación). Reiniciar con `/home/archie/iris/serve.sh bf16` cuando el entrenamiento libere RAM o GPU |
| Pausas térmicas | el entrenamiento pausa a 83 °C y sigue a 72 °C; pasa seguido |

**Después del generador**, `train-local.sh` corre `calibrate` y `eval` (`~/wanda-lncm/eval-small.json`) y escribe `done: checkpoint …`. Ese texto dispara el watcher del LLM.

**Generador confirmado corriendo sin OOM (23:2x):** paso 50 en 41.6 s, es decir ~0.83 s/paso. Los 60 000 pasos tardan unas **14 h** más las pausas térmicas, así que termina alrededor del 2026-10-21 al mediodía o la tarde. A esa altura la loss iba en 5.23 (compose).

Antes de esa medición, la velocidad estaba pendiente. Con micro-lotes de 2, cada paso hace 4 pasadas, así que puede tardar muchas horas.
- Mirar los `seconds` de los primeros registros en `train-log.jsonl`.
- Si es demasiado lento, se puede probar `WANDA_GEN_BATCH=3 WANDA_GEN_ACCUM=3` (pico medido con lote 3: 5.3 GB, pero cambia el lote efectivo a 9) o bajar los pasos.

## 3. Repos, ramas y lo que se hizo

**Repo:** `PentaVM/ArchieOS-Nvidia`. **Worktrees en el Escritorio**:

| Worktree | Rama | Contenido |
|---|---|---|
| `ArchieOS-Nvidia-wanda` | `train/generator-accumulate` (pusheada) | **de acá sale el binario de entrenamiento**; base `13f1187` = `origin/codex/wanda-context` + commit `8eefafd` |
| `ArchieOS-Nvidia-wanda-perf` | `perf/wanda-runtime` (pusheada) | optimizaciones de Wanda + planes + el mismo fix (`c251cee`). **PR todavía no pedido** |
| `ArchieOS-Nvidia-esfera` | `feat/esfera-neuronal-nativa` (pusheada, `e187986`) | esfera nativa de Xavi. **El dueño crea el PR a mano** con `PR-esfera.md` |
| IRIS | `claude/laughing-lamport-5af9cs`, clon en WSL `/home/archie/iris/ArchieOS-Nvidia` | runtime Rust de IRIS (`core/crates/archie-iris-runtime`), todavía no está en main |

### 3.1 `perf/wanda-runtime` (commit `79dc49d` y siguientes)

**LNCM** (`core/crates/archie-wanda-lncm`):
- `nn.rs`: `rms_norm` y `softmax_last_dim` fusionados cuando no hay gradiente y vale la pena. `fused_pays` exige GPU o al menos 32 768 elementos, porque en CPU los tensores chicos van más lentos fusionados.
- `attend`: sin la copia `K.t().contiguous()`.
- `model.rs`: `quick_after_gate` (no repite el gate). `compose` usa `generator.generate_many`: todos los candidatos en una pasada por nivel, con el KV del prompt compartido. Test `batched_candidates_match_one_at_a_time`.
- `tests/bench.rs` (`#[ignore]`): latencia y asignaciones por punto de entrada.

**archie-wanda:**
- `parallel_queries` (1..8) en la config.
- La consulta espera hasta 25 s por un permiso en vez de devolver 503 enseguida.
- `SocialCache` en memoria: ya no parsea todo el Vault por pregunta.
- Lectura acotada: 48 turnos inline y 480 en el tick proactivo, sin perder mensajes. Test `a_bounded_read_takes_the_earliest_and_loses_nothing`.

**Benchmark en CPU** (antes → después):

| Punto de entrada | Antes | Después |
|---|---:|---:|
| quick | 211 ms | 165 ms |
| compose ×1 | 504 ms | 393 ms |
| compose ×4 | 1559 ms | 1185 ms |
| update | 883 ms | 632 ms |
| respuesta completa | 716 ms | 596 ms |
| copias al host | 796 MiB | 585 MiB |

**Docs:**
- `_dev/WANDA-PERF-2026-10-20.md`: auditoría de memoria, caché, paralelismo y contexto.
- `_dev/PLAN-IRIS-NCM-CORTEX-ACCEL-2026-10-20.md`: plan anterior.
- **`_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`: el plan vigente** (commit `4ab578d`), resumido en §4.

### 3.2 Contexto del LLM de Wanda (medido y leído)

- Bonsai-2-9B es un híbrido qwen35: 32 bloques, atención completa cada 4 (solo 8 capas tienen KV), el resto Gated DeltaNet. Se entrenó hasta 262 144 tokens.
- Costo del KV: 32 KiB/token en f16, unos 17 KiB/token en q8.
- En CPU: 405 s para un prompt de 2k tokens. **El LLM es el cuello de botella de Wanda, no el LNCM.**
- `llama-server` en GPU dice **"cache_reuse is not supported by this context"** para este modelo híbrido: `--cache-reuse` no sirve. Hay que probar el guardado de slots en disco (`--slot-save-path`) y verificar que incluya el estado recurrente.

### 3.3 Fix de hoy: acumulación de gradiente (`8eefafd` / `c251cee`)

Pico medido del generador, con la GPU vacía:

| Lote | Pico de VRAM |
|---:|---:|
| 8 | OOM |
| 4 | 7.5 GB |
| 3 | 5.3 GB |
| 2 | 4.0 GB |

La causa es la atención sobre 768 + 340 posiciones, sin flash-attention.

- Nueva opción `--accumulate N` (en `train.rs`, función `task_step`, con `add_gradients` y `apply_gradients`), para las etapas de generador y adapter. Con N = 1 el paso es idéntico al de antes, incluidas las extracciones aleatorias.
- Test nuevo `accumulated_halves_give_the_whole_batch_gradient`; el adapter del pipeline corre con `accumulate: 2`. Pasan los 59 tests unitarios y el pipeline.
- `train-local.sh`: el preset small usa `GB=2 GA=4`, y se puede cambiar con `WANDA_GEN_BATCH` / `WANDA_GEN_ACCUM`.

## 4. Findings de IRIS y plan (resumen de `_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`)

### 4.1 IRIS Beta 2.1

**Arquitectura** (pesos `iris-beta21-step132000.pt`, sha `c6cadb02…3b97`, en `/home/archie/iris/`):
- canvas 512, es decir 1024 tokens;
- V-JEPA 2.1 ViT-B (166 MiB) + SigLIP2 visión (178 MiB);
- fusión de 6 capas, d 512, 8 cabezas, con un **MoE de 32 expertos top-2** (512→2048→512), capacidad 1.5 y un experto compartido. **Los expertos pesan 769 MiB, el 57 % de los pesos**; cada uno ocupa 4 MiB;
- DETR de 100 consultas;
- cabeza de acción T1–T4 (`act_chunk` 8);
- cabezas de lectura, máscara, keypoints y temporal **entrenadas en la release pero no portadas a Rust**;
- `read_head: true`, `text_tokens_per_phrase` 32. IRIS no tiene difusión.

**Medido en la 3070, bf16:**

| Modo | Tiempo | fps |
|---|---:|---:|
| open (256 frases, 17 filas) | 517 ms | 1.93 |
| focused | 138 ms | 7.2 |
| en vivo con barrido de 2 s | — | 5.5 |

- VRAM: unos 3 GiB.
- **Estimado:** ronda los 4 TFLOPS efectivos, cerca del 10 % del pico de la tarjeta.

### 4.2 Puntos críticos (exactos: la paridad la verifica `tests/parity.rs`)

- **C4 — bug en producción** (`siglip.rs:212`): falta `.contiguous()` en el vector pooled. Por eso falla en CUDA codificar 2 o más frases nuevas en lote. **Es una línea; arreglarlo primero.**
- **C1 — el MoE rutea en la CPU** (`model.rs`, `SparseMoe::forward`): por capa hace `to_vec1`, ordena en el host y hace unas 65 copias con sincronización.
  - Hay que portar el algoritmo Python sin sincronización: `topk`, `one_hot().cumsum()` para las ranuras y `baddbmm` sobre `[E, C, D]`.
  - Para lotes con varios focos, calcular la capacidad por imagen.
- **C2 — atención sin fusionar**: guarda los puntajes completos, pasa bf16→f32 para el softmax y aplica máscaras de tamaño completo. Solución: `candle-flash-attn` (sm86), que además permite quitar `in_slices`.
- **C3 — todo va a la GPU** (`weights.rs:159`): incluso cabezas que no se usan, y la torre de texto SigLIP2 (unos 565 MiB).
  - Cargar en GPU solo lo que se usa.
  - Correr la torre de texto en CPU, en f32.
  - Guardar la caché de frases en SSD.
- **C5 — el modo abierto repite la imagen 17 veces**: la autoatención de la capa 0 es idéntica en todas las filas; calcularla una vez.
- **C6 — `decode`** sincroniza con el host: usar `topk` en GPU.
- **C7 — `instance_embeddings`** baja el mapa entero (2 MiB) por cada detección y hace `roi_align` en CPU. `roi_align` es lineal, así que se puede hacer como un matmul con pesos bilineales armados en el host.
- **C8 — `/act` y `/detect` codifican dos veces el mismo frame.** `encode_fused_state` (fusión con la frase nula) es el mismo tensor que usan la memoria de `/act`, la lectura, el modelo temporal y el seguimiento. Publicarlo una vez por frame.
- **C9 — preparación del frame en el bucle caliente** (JPEG, letterbox): hacerla en un pipeline de CPU con doble buffer.
- **C10 — RoPE y `patchify`** hacen copias; fusionarlas.

### 4.3 Plan por fases (metas a verificar, no promesas)

| Fase | Contenido | Meta |
|---|---|---|
| F0 | medir: nsys, histogramas de ruteo de expertos por fuente, VRAM, tiempo de host, PCIe | — |
| F1 | C1, C3–C9 (exactos) | open ≤ 300 ms, focused ≤ 90 ms, −0.6 GiB |
| F2 | flash-attention y ops fusionadas; opcional, TensorRT para los codificadores | focused ≤ 50 ms |
| F3 | **`archie-accel`** (ver abajo) | — |
| F4 | **S-LINKs** (ver abajo) | — |
| F5 | entrenamiento con el tronco congelado (ver abajo) | — |
| F6 | megakernel persistente solo donde F0–F3 muestren que el límite son los lanzamientos (LNCM small, cabezas chicas) | — |

**F3 — `archie-accel`.** Un único dueño de la GPU para los modelos candle (IRIS y LNCM):
- lotes con varios focos;
- prioridades por stream;
- residencia de expertos en GPU / RAM / SSD, con prefetch mientras corren los codificadores. Un experto no residente equivale a capacidad 0: sus tokens van al experto compartido, que es la degradación que el modelo ya vio en entrenamiento. Hay que medirla con A/B;
- presupuesto de VRAM negociado con `llama-server`;
- trabajo disparado por daño en la pantalla, no por reloj.

**F4 — S-LINKs y puente OCR.**
- S-LINKs: canales tipados, versionados y sin espera (seqlock / eventos CUDA). Llevan datos, nunca autoridad.
- Enlaces: `scene.state`, `scene.text`, `scene.people`, `social.intent` (nivel grueso del VQ de Wanda), `social.focus`, `llm.partial`, `task.intent`, `focus.plan`. Ver la tabla en el plan.
- Join del post-procesado en tanda: la latencia total pasa a ser el máximo de los modelos, no la suma.
- Puente OCR, etapa A:
  1. Fuentes de texto en orden: accesibilidad/DOM → `read_decoder` portado con caché de KV → OCR externo.
  2. Las líneas cambiadas, en lote, van al `update` del LNCM y modifican el `SocialState`.
- En Cortex: `ResourceAdvice` como payload de `CandidateSchedulerChange`, más telemetría.

**F5 — entrenamiento con el tronco congelado** (mismo SHA de IRIS):
- Cabeza **IRIS-LNCM de acción**: una skill es estado (vectores en el prompt), no LoRA. Así se puede batchear entre skills.
- `read_decoder` reentrenado para emitir **ids del códec de Wanda**, que entran directo a `embed_prompt` del LNCM.
- **LNCM v2** con un segmento `<scene>` que tiene su propio KV (máscara por bloques), RoPE y `prompt_len` de 2048–4096. El prompt actual es bidireccional y no tiene máscara, así que cualquier cambio invalida todo su KV.

**Autoridad, sin cambios:** el contrato de Cortex dice que `predict_process` **no** puede cambiar política, prioridad ni grants. Toda acción pasa por Cerebra → Policy → lease → Stage.

## 5. Qué iba a hacer después, en orden

1. **Vigilar el generador:**
   - medir la velocidad con los primeros registros;
   - confirmar que no hay OOM: debería rondar los 4 GB;
   - re-armar el monitor cada 30 min (`scratchpad/watch-train3.sh` o el equivalente).
2. Cuando el entrenamiento termine bien:
   - revisar `~/wanda-lncm/eval-small.json`;
   - confirmar que el watcher subió el LLM a la GPU (`/home/archie/wanda/llm-after-training.log`);
   - medir el LLM en GPU (tokens/s, contexto de 64k y 4 slots);
   - reiniciar IRIS (`/home/archie/iris/serve.sh bf16`).
3. **IRIS F1**, en la rama de IRIS, desarrollando en CPU con `tests/parity.rs`:
   1. C4 (una línea);
   2. C1 (MoE en GPU);
   3. C3 + C8;
   4. después, el perfil F0 en la 3070.
4. Pedirle al dueño que abra los PRs: `perf/wanda-runtime` (incluye el fix y los planes) y el de la esfera (`PR-esfera.md`).
5. Pendientes del benchmark de IRIS: fp32, la línea base en Python y la prueba con una persona y una taza. La GPU se cayó el 2026-10-16 (`CM_PROB_PHANTOM`) con carga combinada: poner límite de potencia y guardián térmico antes de cargar todo junto.

## 6. Archivos de referencia

- Plan vigente: `Desktop\ArchieOS-Nvidia-wanda-perf\_dev\PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`
- Auditoría de Wanda: `Desktop\ArchieOS-Nvidia-wanda-perf\_dev\WANDA-PERF-2026-10-20.md`
- Benchmark de IRIS: `/home/archie/iris/IRIS-RTX3070-BENCH.md` (en WSL)
- Código de IRIS para entrenamiento (Python): `/opt/archie/iris/src/archie_iris_foundry`. Venv: `/opt/archie/iris-venv`.
- Datos y checkpoint de Wanda: `~/wanda-lncm/` (`data/`, `ckpt-small/`, `logs/`)
- Modelo LLM: `/home/archie/wanda/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf`. `llama-server` está en `/opt/archie/llama-cuda/bin/`.
- Memoria persistente de Claude: `C:\Users\Penta\.claude\projects\C--Users-Penta-Desktop-ArchieOS\memory\`

## 7. Revisión del PR #47 (IRIS C4 + C1, hecho por otra sesión), 2026-10-21

PR: https://github.com/PentaVM/ArchieOS-Nvidia/pull/47, rama `dev/gracious-franklin-hu2brn`, apilado sobre el #35.

**Lo que está bien:**
- **C4** (`.contiguous()` en `siglip.rs`): correcto.
- **C1** (`route_on_device`): reproduce la semántica de Python. Revisado punto por punto:
  - top-k con desempate al experto menor;
  - posiciones en cola en orden (token, k), con `inclusive_counts` por bloques: exacto en f32 mientras candle no use TF32, que es su valor por defecto;
  - los desbordes van a una fila de reserva con aporte cero;
  - la suma por token se hace en orden ascendente de experto, igual que la ruta en el host.
- Mantener la ruta en el host para CPU es buena decisión, y está medida.
- Dejar la capacidad por imagen para F3 también es correcto.

**Bug encontrado y confirmado:** en la release, `b_in` y `b_out` tienen forma `[32, 1, H]`, no `[32, H]`.
- `route_on_device` hacía `narrow(..).unsqueeze(1)`, lo que da `[E, 1, 1, H]`, y el matmul siguiente falla. O sea, **el camino CUDA no corría con los pesos reales**.
- Los tests no lo detectaban porque construían sesgos 2-D.
- Con sesgos de la forma real fallan 2 de 3 tests; con el arreglo (`reshape((count, 1, ()))`) pasan los 3.
- El arreglo está en la rama **`fix/pr47-moe-bias-shape`** (commit `9fc97b9`, encima de `cb1619b`) y como parche en `pr47-moe-bias-shape.patch`. Falta aplicarlo al PR: `git cherry-pick 9fc97b9` en la rama del PR.

**Sin verificar en el PR** (lo dice el propio PR): la compilación con `--features cuda`, `tests/parity.rs` sobre los pesos reales y los tiempos en la 3070.
