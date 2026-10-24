# Wanda + IRIS: estado de la implementación — 2026-10-21

Rama `integrate/wanda-iris` = `main` (con #35) + `perf/wanda-runtime` (Wanda) + `dev/gracious-franklin-hu2brn` (IRIS, PR #47) + lo de este día. Todos los commits son de PentaVM. Este documento reemplaza la lista de pendientes de `_dev/IMPLEMENTACION-IRIS-ACCEL-SLINKS-2026-10-21.md` y de `_dev/handoff/2026-10-20/HANDOFF.md` en lo que toca a Wanda e IRIS.

Cada afirmación lleva cómo se verificó:
- **medido**: en la RTX 3070 o en la WSL `ArchieOS-Operator-RTX3070-Test`;
- **prueba**: hay una prueba automática;
- **sin probar**: compila, pero no corrió donde importa.

## 1. Qué hay

### Wanda (LNCM)

| Parte | Estado | Verificación |
|---|---|---|
| Runtime del LNCM optimizado (fusiones, lotes de candidatos, caché del estado social, lecturas acotadas) | hecho | **medido** en CPU: respuesta completa 716 → 596 ms; pruebas del crate |
| Acumulación de gradiente (`--accumulate`): el generador entra en 8 GB | hecho | **medido**: lote 8 = OOM; 4 × 2 corre, con pico de 4 GB; prueba de equivalencia de gradientes |
| Voz con Wanda sobre el ruteo y el veil de #35 | hecho | **prueba** `the_routed_voice_offers_wanda_only_if_every_restricted_choice_is_loopback`; clippy |
| Texto de pantalla → estado social (`read_scene`, `SceneSink`, `POST /v1/scene/text`) | hecho | **prueba** en `tests/lncm.rs`; **medido** en vivo con la demo |
| Entrenamiento del generador | en curso | llegó al paso 46 700 de 60 000; un corte por disco lleno lo obligó a retomar desde el guardado del 46 000 a las 17:43 del 2026-10-21. Pausas por temperatura a 85 °C. Al ritmo medido (~3 000 pasos/h con pausas), termina cerca de las 22:30 (estimado) |
| Respuestas exactas desde el estado (`exact.rs`): "¿sabe X…?", "¿qué cree X…?" y expectativas que la pregunta nombra | hecho | **prueba**: en 32 mundos sintéticos, todas las preguntas de "sabe" y "cree" se responden y coinciden con el simulador. **Medido** con el checkpoint del paso 48 000 en CPU, sobre 600 preguntas de desarrollo: sabe 0.740 → 1.000, cree 0.285 → 1.000, expectativas 0.620 → 0.835 |
| Cadena de respaldo de Wanda: estado exacto ("sabe", "avisar", "cree", "qué tener en cuenta") → cabeza de copia (`pointer.rs`) → verificación contra el estado (`verify.rs`: valores corregidos; temas, rasgos inventados y respuestas que no responden, rechazados) → n-gramas (`ngram.rs`: ruido rechazado) | hecho; la cabeza de copia **sin entrenar** todavía (se entrena sobre el generador final) | **prueba** de punta a punta en el preset tiny (entrena, carga, sirve y rechaza archivos alterados). **Medido**, calificado por Claude sobre los mismos 30 casos con el checkpoint del paso 54 000 (sin la cabeza de copia): Wanda 9.3 → 18.2 de 20, errores graves 21 → 0; Nemotron 3 Ultra, 17.7. Esta segunda calificación no fue a ciegas, y lo que habla hoy es sobre todo el estado, no el generador |
| Evaluación del generador (paso 46 000, 200 ejemplos por tarea) | — | **medido**: gate y sí/no 1.0; detección de cambio 0.995; redacción F1 0.733 (subiendo); decisión 0.55 (estancada desde el 30 000); ECE 0.082; **escritura del estado (`update_line_f1`) 0.01, el punto débil**. La redacción se compara con las plantillas propias del simulador, no con un maestro: contra 200 respuestas de Nemotron 3 Ultra da 0.27 (paso 48 000, CPU), cuando dos respuestas de Nemotron al mismo caso se parecen 0.51 y las plantillas 0.31 |

### IRIS

| Parte | Estado | Verificación |
|---|---|---|
| C1–C10: MoE en GPU, pesos a demanda, torre de texto en CPU, caché de cuadro, top-k en GPU, roi_align como matmul, RoPE en una pasada, flash-attention opcional | hecho (PR #47 + el arreglo de sesgos `[E,1,H]`) | **prueba** de paridad por ruta; **medido**: enfocado 132 ms (antes 138), abierto 690 ms con la tarjeta limitada a 132 W |
| Pipeline en vivo (`/live`): responde cada cuadro con daño + seguimiento | hecho | **medido**: 60 cuadros/s respondidos, p90 2.5 ms, 6.5 lecturas del modelo/s |
| archie-accel (residencia de expertos, plan por foco, sobre de Policy) | hecho | pruebas del crate; la residencia está apagada por defecto |
| S-LINKs (`archie-slink`), `ResourceAdvice` en Cortex, slots de llama-server | hecho | pruebas; los slots no se verificaron con el estado DeltaNet de Bonsai |
| Cabeza de lectura propia (`read_decoder`) portada, `POST /read` | hecho | **prueba** de paridad con Foundry: relativo 6.2e-7 y los mismos tokens. **Lee mal pantallas y papel**: reemplazada por el experto OCR |
| **Experto OCR: GOT-OCR 2.0 dentro de IRIS** (`ocr.rs`, `POST /ocr`, `--ocr-model`) | hecho | **prueba** de paridad con transformers 5.20 (ver §2) |
| Visor nativo `archie-iris-visor` (diseño de `AppWindow.qml`) | hecho | **medido** en vivo con la Elgato |
| Librería de cámara `archie-camera` (V4L2 del kernel + puente) | escrita, sin conectar | **prueba** de la ABI contra `videodev2.h`; sin dispositivo real |

## 2. El experto OCR sin entrenar

El lector propio de IRIS (8.6 M parámetros, caracteres, recortes de calle) devolvía basura en pantallas y papel: `'www.je'`, `'lexprating'`. En vez de entrenar, IRIS corre un OCR ya entrenado como experto propio, en su mismo proceso y su misma memoria:

- **Modelo:** GOT-OCR 2.0 (`stepfun-ai/GOT-OCR-2.0-hf`, revisión `d3017ef2`, Apache-2.0). Son 560 M parámetros: un ViT-B tipo SAM (ventanas 14 × 14, posiciones relativas, capas globales 2/5/8/11), un neck y un proyector convolucionales que dejan 256 tokens, y un decodificador Qwen2 (24 capas, ancho 1024). SHA-256 de `model.safetensors`: `6175ac78…3ba4`, igual al del Hub.
- **Port:** a Rust/candle con las mismas piezas de `nn.rs` de IRIS, sin dependencias nuevas.
- **Lectura por regiones:** la imagen se codifica una vez por cuadro y cada caja es un prompt aparte (`[x1,y1,x2,y2] OCR:` en la grilla 0–1000), que es la lectura fina con la que se entrenó.
- **Paridad con `GotOcr2ForConditionalGeneration`** (CPU, f32, `tests/ocr.rs` + `tests/reference/got_reference.py`), sobre dos líneas en español:

  | Comparación | Resultado |
  |---|---|
  | ids del prompt | idénticos |
  | tokens de imagen | relativo 1.0e-5 |
  | logits después del prompt | relativo 3.3e-6 |
  | lectura | idéntica: "ArchieOS ve la pantalla / Reunión mañana 18:30" |
  | canvas bicúbico propio | diferencia media 2.4e-4 con el del procesador; misma lectura desde el PNG |

- **Tiempos en CPU** (16 hilos, f32): codificar 21 s y leer 4.5 s. Son tiempos de prueba. **Sin medir en GPU**: la GPU entrena a Wanda. La expectativa (no medida) es de cientos de ms por lectura en bf16.
- **Visor:** con `--ocr-expert` usa este experto en lugar de Qwen3-VL. GOT lee y no ubica, así que las líneas van a Wanda sin caja.

## 3. Lo que falta

**Necesita la GPU** (cuando termine el generador de Wanda, o pausándolo):
1. Medir el experto OCR en bf16, y el visor con `--ocr-expert` en vivo.
2. Perfil por etapas de IRIS (`IRIS_PROFILE=1 archie-iris-bench`): dónde van los 56 ms de fusión y decodificación.
3. Build con `--features flash-attn` y medirlo.

**Necesita entrenamiento** (el tronco de IRIS queda congelado):
4. La cabeza IRIS-LNCM de acción (skills como estado, no LoRA).
5. El LNCM v2 con segmento de escena.
6. Opcional: destilar GOT en el `read_decoder` si se quiere un lector más chico que 560 M.
7. Si la evaluación final de Wanda (500 ejemplos) deja `update_line_f1` cerca de 0: una etapa corta del generador con la mezcla cargada hacia `update`.

**Código sin GPU:**
8. Wanda publica `social.intent` mientras responde; el harness consume `task.intent`.
9. Conectar `archie-camera` al visor y un emisor de Windows en Rust en lugar de `camera_bridge.py`. Necesita el toolchain de Rust en Windows o el target `x86_64-pc-windows-gnu`.
10. Verificar que los slots de `llama-server` guardan el estado recurrente de Bonsai (DeltaNet).

**Del dueño:**
11. Abrir el PR de `integrate/wanda-iris` contra main, o pedir que se abra.
12. Voz (Voxtral 3) y Sentry: sesiones aparte. La de Sentry se cortó por el límite de uso.

## 4. Cómo levantar la demo

Scripts en el scratchpad de la sesión: `demo-start.sh`, `visor-attention.sh` y `demo-stop.sh`.

1. Pausar el entrenamiento: `touch ~/wanda-lncm/ckpt-small/STOP`. Guarda el paso.
2. Levantar IRIS en bf16, agregando `--ocr-model /home/archie/models/got-ocr2` para el experto OCR.
3. Levantar Wanda con `~/wanda-demo/wanda.toml`. Usa una copia del checkpoint, con el generador registrado en su manifiesto.
4. Abrir el visor con `LD_LIBRARY_PATH=/usr/lib/wsl/lib GALLIUM_DRIVER=d3d12`, que es el driver de GPU de WSLg; sin eso dibuja en CPU al 850 %.
5. Arrancar el puente de la cámara en Windows.

Al terminar, reanudar el entrenamiento con `Desktop\ArchieOS\wanda-train-start.sh`.
