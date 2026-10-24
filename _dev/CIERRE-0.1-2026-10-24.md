# Cierre 0.1 — 2026-10-24: GUI recorrida, Bonsai 27B en caliente, voces, Wanda y main

Rama `claude/unified-0.1` en `PentaVM/ArchieOS-Nvidia`, PR #51 hacia `main`. Nodo de
prueba: WSL `ArchieOS-Operator-RTX3070-Test`, con una RTX 3070 de 8 GB.

Quedó fuera: el HUD del World con Aleph. Es trabajo de Codex, en
`codex/world-geofabric-rust`, y no se tocó.

## 1. La GUI, control por control

`archie-display --shell --ui-walk` (`display/native/src/UiWalk.h`) clickea los controles
que lista `display/native/tests/ui-walk-shell.json`. Por cada clic guarda una captura y el
estado del shell antes y después. Contra el nodo vivo dio 293 clics, 60 sin efecto visible
y 10 sin destino. Están clasificados en `_dev/verification/ui-walk-2026-10-24/README.md`:
ninguno es un control de mentira.

Tres entradas de menú que quedan sobre la barra de título de una ventana fallaban solo con
el envío directo del recorrido. Por la entrada de Qt sí funcionan, y lo prueba
`tst_MenuOverWindows.qml`. `tst_SettingsOptions.qml` clickea cada opción del orbe y de la
apariencia, incluidas las que el recorrido no alcanzó bajo el pliegue.

Lo que se cambió para que no quedaran controles de muestra (`d79e91a`):
- Vault es un gestor de archivos real, de solo lectura.
- Forge lanza las aplicaciones del catálogo.
- El Centro de control y Ajustes → Autoridad usan la pausa real de Core.
- Se sacaron «Bloquear pantalla» y los archivos de muestra.

Resultados: QML 84/84 y ctest 4/4 offscreen.

## 2. Bonsai 2 27B y el cambio de modelo en caliente

- Pesos: `Ternary-Bonsai-2-27B-PTQ1_0.gguf`, 5 946 648 928 B, sha256 `53107f53…33ee3` (el
  mismo que en Hugging Face). Están en `D:\ArchieLocal\models`, con un enlace desde
  `/var/lib/archie/models/local`.
- Runtime: PrismML llama.cpp **b10787** (`73bfe6cc4`, release CUDA 12.8, tarball sha256
  `f61ac818…`). Desde hoy es `/opt/archie/llama-cuda`. El b10777 queda al lado como
  `llama-cuda.b10777`.
  - `--fit` puso 63 capas del 27B en la GPU (`llama-fit-params`: `-ngl 63`).
  - `--reasoning-effort-allow none,medium,xhigh` contesta un «high» en medium en vez de un 500.
- `archie-local`: `list [--json]`, `use 27b|9b|4b|1.7b`, `current`, `setup <modelo>`,
  `up`, `down` y `status`. Sirve uno a la vez. El router de Core solo le pregunta a un modelo
  llama.cpp mientras responde `/health`, así que cambiar no reinicia nada. Ajustes →
  Modelos lo hace con «Usar» (ShellRuntime; captura en `_dev/verification/`).
  `modelo-local-archie.bat 27b|9b` lo hace desde Windows.

Medido en la RTX 3070 (scratch `bench-llm.sh`, `llm-timing.py`):

| | 9B (b10777) | 9B (b10787) | 27B (b10787, 63 capas en GPU) |
|---|---|---|---|
| Generación | 120 tok/s | 113–125 tok/s | 41–43 tok/s |
| Lectura de 3 649 tokens | 2 352 tok/s | 2 296 tok/s | 691 tok/s |
| Pregunta corta, directa | 0,47 s | 0,40 s | 1,4 s |
| Razonando | 9B *on*: el pensamiento gastó los 1 024 tokens, sin respuesta | igual | *medium*: 14,9–15,8 s (una de tres llegó al tope de 1 024 tokens: 24,8 s) |
| Carga | segundos | segundos | 2 min 26 s desde D: |

Pregunta de zona horaria: «Santiago a las 21:00, ¿qué hora es en Tokio?». La respuesta
correcta es 9:00 del día siguiente.
- En el banco, solo el 27B en *medium* acertó.
- Por Core, en otra pasada, el mismo 27B dijo 12:00.

Son muestras sueltas, no una medida de calidad.

Por Core (`archie --json ai ask --privacy restricted`, sin fijar modelo), sin reiniciar
archied ni Wanda:
- Con el 27B arriba, la respuesta vino de `local.bonsai-2-27b-fast` (1,7 s).
- `archie-local use 9b` tardó 5 s.
- La misma pregunta siguiente vino de `local.bonsai-2-9b-fast` (0,4 s).

Wanda por Core, con el checkpoint nuevo (sección 4):

| Modelo | Consulta normal | `deep` |
|---|---|---|
| 9B | 0 recomendaciones (9 s) | `unverified_advice`: el JSON no pasó la verificación |
| 27B | recomendación con evidencia del Vault y recibo de Core (32 s) | también, con alternativas y condición de revisión (44 s) |

**El 27B queda como el modelo elegido del nodo.**

## 3. Voces y los dos idiomas

Probé once voces Piper de licencia abierta. Cada una leyó tres frases, y Nemotron ASR (el
mismo reconocedor de `archie-talk`) escribió lo que oyó. WER = palabras mal oídas, en
promedio; RTF = tiempo de síntesis sobre duración del audio.

| Voz | Idioma | WER | WER con `auto` | RTF | Licencia |
|---|---|---|---|---|---|
| **es_MX-claude-high** | es | 0 % | 0 % | 0,31 | Apache-2.0 |
| es_MX-ald-medium (antes) | es | 2,8 % | 2,8 % | 0,29 | Unlicense |
| es_AR-daniela-high | es | 9,1 % | 9,1 % | 0,63 | CC BY-SA |
| **en_US-kristin-medium** | en | 0 % | 0 % | 0,35 | dominio público |
| en_GB-cori-high | en | 0 % | 0 % | 0,54 | dominio público |
| en_US-norman / bryce / joe | en | 3,3 % | 3,3 % | 0,32–0,38 | DP / DP / CC0 |
| en_US-john-medium (antes) | en | 6,7 % | 6,7 % | 0,38 | dominio público |

- Quedaron es_MX-claude-high y en_US-kristin-medium, por medición. La model card de
  Kristin dice que es una voz femenina. La de claude-high no lo dice, pero su tono medido
  (192 Hz de mediana) cae en rango femenino.
- `install-piper-voices.sh` las baja de una revisión fijada de piper-voices y verifica su
  md5. `run-archie.sh` las usa si están.
- Con `LISTEN_LANGUAGE=auto`, que `archie-talk` trae por defecto (`--spanish` para solo
  español), el reconocedor oyó ambos idiomas igual que con el idioma fijo.
- La prueba de punta a punta, con WAV y el 9B local, encontró un fallo: la pregunta en
  inglés se respondía en español, aunque la persona en español decía que se contesta en
  el idioma en que te hablan.
- Ahora, si la transcripción está en inglés, el turno lo dice en inglés, en el system y
  junto a lo dicho; la historia guarda las palabras como se dijeron. Después del cambio:
  - «What time is it in Tokyo…?» → «It is 2:43 AM in Tokyo.», con la voz en inglés.
  - La pregunta en español → respuesta en español.
  - La hora sigue mal, pero eso es el 9B.

## 4. Wanda: la última etapa de entrenamiento

`wanda-final-stage.sh` (en `Desktop\ArchieOS`) partió de `~/wanda-final`:
1. Generador hasta 66 000 pasos, con la mezcla cargada hacia `update`
   (gate 0.05, update 0.6, decide 0.2, compose 0.15; lr 1e-4).
2. Cabeza de copia de 3 000 pasos.
3. n-gramas y calibración.

Duró de 07:09 a 10:15, con pausas térmicas a 80–83 °C. Evaluación: 500 ejemplos por tarea,
mismo binario.

| Métrica | antes (`~/wanda-final`) | después |
|---|---|---|
| update_line_f1 | 0.0262 | 0.0291 |
| compose_char_f1 | 0.7486 | 0.7427 |
| decision_accuracy | 0.4740 | 0.4760 |
| abstain_recall | 0.8276 | 0.7931 |
| change_accuracy | 0.992 | 0.990 |
| gate_accuracy | 1.0 | 1.0 |

La regla estaba fijada de antemano: `update` mejora y el resto queda dentro de 0.02. Dio
**adoptar**, y el checkpoint está en `~/wanda-demo/lncm`. El anterior quedó en
`lncm.before-20261024`. sha256:
- `generator.safetensors`: `bf600f9a…f231`
- `pointer.safetensors`: `77b3eb95…57e4`

Dicho sin adorno: la mejora es chica y `update` sigue cerca de 0. Esta etapa era el punto 7
de `ESTADO-WANDA-IRIS-2026-10-21.md` y no la resolvió. Escribir el estado necesita otro
enfoque (datos o formato de la tarea), no más pasos de la misma mezcla. Queda para después
de 0.1.

Datos:
- `~/wanda-lncm/data/synth/examples.jsonl`
- `~/wanda-lncm/data/distilled.jsonl`
- Registro de entrenamiento: `~/wanda-update-20261024/train-log.jsonl` y `run/`.

## 5. El orbe

La fase 1 del port de Lumenfield entró (`ab70d15`): los perfiles salen del catálogo de
Xavi. A las 10:00 no había una rama de diseño más nueva (`design/lumenfield-0449-orbe`).
Este es el orbe que sale.

## 6. Validación y merge

Desde el 2026-10-22, GitHub no corre los jobs. Rechaza cada uno con «recent account
payments have failed or your spending limit needs to be increased». Así que el CI de
`ci.yml` se corrió aquí, en WSL, crate por crate, borrando los ejecutables de prueba para
no llenar C:. Resultado:

| Paso de `ci.yml` | Aquí |
|---|---|
| `cargo fmt --all -- --check` | limpio |
| `cargo clippy --workspace --all-targets -- -D warnings` | limpio (todo el workspace) |
| `cargo test`, por crate | 21 crates, 0 fallas: a11y, accel, arena, bridges, camera, cerebra, cli, core (162), cut, desk, displayd, execution, executord (131), fabric, geofabric, installer, iris, iris-bridge, iris-runtime, iris-visor. Además model-runtime, voice (65) y wanda, después de los cambios de hoy, y lumen-design (26). |
| Nativo: cortex, sentry, display/visual, display/osm | 4/4, 19/19, 19/19 y 2/2 |
| `tshark_e2e` de archie-lan | no corrido: no hay tshark aquí. archie-lan no cambió desde su CI verde (`ad35da4b`). |

El recorrido por crates se detuvo después de archie-iris-visor: C: bajó a 1,1 GB. El
guardián de disco no actuó, porque `df -BG` redondea hacia arriba y leyó «2»; ya está
corregido y mide en MB. Los crates que faltaron no cambiaron desde corridas completas que
pasaron:
- `cargo test --workspace` local del 2026-10-23 (`HARNESS-PAUSA-VIVACIDAD-2026-10-23.md`):
  2 878 pasan, 0 fallan. Ya incluye los últimos cambios de memory, postgres, router,
  state y workstation.
- Los CI verdes del 21 en GitHub.

Lo único que cambió después tiene arriba sus pruebas de hoy.

## 7. Disco

- D: está lleno (466 GB; 0,3 GB libres). Lo llenó la biblioteca de juegos, no ArchieOS:
  `D:\ArchieLocal` ocupa 11 GB.
- C: quedó cerca de 3 GB. El vhdx de esta distro no devuelve espacio con `fstrim`
  (recortó 3,8 MiB), pero ext4 reutiliza lo borrado dentro. Las compilaciones grandes
  necesitan antes espacio en C: o en D:.

## 8. Pendiente, sin vueltas

- Habilitar el pago de GitHub Actions, para que el CI vuelva a correr en GitHub.
- `update` de Wanda: otro enfoque.
- Validar el router con un dispositivo real.
- El aviso hablado de Sentry: falta Npcap en Windows.
- ISO/OVA.
- La conexión de cámara, micrófono y voz que pidió el port de Lumenfield: viene con el
  diseño de Xavi.
