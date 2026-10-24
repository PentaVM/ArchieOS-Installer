# Harness²: etapas JSON con decodificación restringida por esquema (PR 49) — 2026-10-22/23

Rama `claude/harness2-json-schema` (PR 49, base `claude/unified-0.1` en `1d7f1c2`), en
`PentaVM/ArchieOS-Nvidia`. Cierra el pendiente del §3 de
`ESTADO-UNIFIED-0.1-2026-10-22.md`: en las primeras corridas reales, Bonsai devolvió
una forma de respuesta distinta en cada una (prefijo `harness/`, listas u objetos en
`trajectory`/`notes`, cerca ```json, JSON roto, salto de línea en `guardrails.md`).

## 1. Diseño (commits `ed11d90`, `f4e77c3`, `3b91e24`, `0baa3cd`)

1. **Puente** (`harness2.rs`): `reply_schema(response_format, base)` da el esquema JSON
   de cada etapa JSON, con exactamente las claves que lee el worker.
   - `artifact`: `{artifact, trajectory}`, ambos texto de al menos un carácter.
   - `harness`: `{harness: {los 4 archivos del `ALLOWED` del worker}, notes}`.
     - `guardrails.md` va fijado con un `enum` de un solo valor: el texto exacto de la
       base, porque el worker aborta ante cualquier edición. No se fija si su JSON pasa
       de 8 KiB; entonces queda como texto y el worker lo sigue comprobando.
     - Los demás archivos, texto no vacío; `notes` puede ir vacío.
   - En todos los niveles, `additionalProperties: false` y todas las claves requeridas.
   - `experience` (la reflexión, texto con etiquetas) y `selection` no tienen esquema.
2. **Sustrato** (`harness_runtime.rs`, `stage_reply_schema`): solo un runner API recibe
   el esquema; Codex nunca. El `response_format` de cada etapa ya lo fija `admit_stage`.
3. **Servicio unificado** (`ai.rs`): `InferenceRequest.reply_schema` es
   `#[serde(skip)]`. Lo fija solo código de confianza del mismo proceso; el cable de
   control no puede traerlo.
4. **Runtime** (`archie-model-runtime`):
   - `ChatRequest.reply_schema: Option<serde_json::Value>`, `#[serde(skip)]`; se
     actualizaron todos los literales.
   - En el cable OpenAI va como `response_format: {type: "json_schema", json_schema:
     {name: "reply", strict: true, schema}}`, junto al interruptor de razonamiento.
   - Solo llama.cpp recibe `minLength`/`maxLength`; a los demás servidores se les quitan,
     porque un validador estricto podría rechazarlos.
   - El cuerpo de Anthropic se arma campo por campo y nunca lo lleva.
   - Un esquema que no describe un objeto o que pasa de 16 KiB se rechaza antes de
     conectar.

Lo que no cambia:
- `worker.py`, sus pruebas y `protocol.json`.
- Una edición fuera de la superficie o de `guardrails.md` sigue abortando.
- La promoción sigue denegada por defecto.
- Los lectores tolerantes del worker siguen como respaldo.
- El esquema restringe la forma, nunca el contenido.

## 2. Arreglos de esta validación (sobre `0baa3cd`)

- El comentario de documentación de `seed_harness` había quedado pegado a
  `stage_reply_schema`, insertada entre ambos. Se devolvió a su función.
- En la prueba real, `ARCHIE_TEST_LLAMA_MAX_OUTPUT` (4096 por defecto) quedaba recortado
  en silencio a los 2048 tokens del perfil por defecto. Ahora también sube el techo del
  perfil, como dice su documentación y como en las corridas medidas del 22 (4096).
- La prueba real imprime el recibo de cada etapa (estado, tokens, duración, error) para
  el registro del operador.
- Cobertura portada del trabajo local: las claves de primer nivel de cada esquema quedan
  ligadas a las llamadas `shape(...)` de `worker.py`.

## 3. Validación

Entorno: WSL `ArchieOS-Operator-RTX3070-Test`, usuario `archie`, objetivo
`/var/tmp/archie-pr-integration-target` reutilizado, 2 trabajos, sin debuginfo,
`CUDA_VISIBLE_DEVICES` vacío salvo en el slot de GPU. Logs en
`/var/tmp/archie-logs/claude-schema/`. En esta WSL, `cargo clippy` usa el clippy del
toolchain por defecto de rustup, que es 1.88, el mismo de CI.

| Paso | Resultado |
|---|---|
| `cargo fmt --all -- --check`, rustfmt 1.88 (el de CI) y 1.99 | limpio |
| `cargo clippy` de los 5 crates de PR 49, `--all-targets -D warnings` | limpio |
| `cargo test` de archie-model-runtime, archie-workstation, archie-cerebra y archie-wanda-lncm | 315 pasan, 0 fallan, 23 ignoradas |
| Worker de Harness² en Python (CPython 3.13) | 25/25 |
| `harness2_bridge -- --include-ignored`, upstream fijado real | 9/9 |
| `unified_runtime -- --ignored --skip harness2_completes_on_a_real_llama_cpp_server_under_reply_schemas` | 11/11 |

PostgreSQL:
- Clúster PG18 desechable propio en 127.0.0.1:55441, con TimescaleDB 2.29.2,
  pgvector 0.8.6 y autenticación `trust`, así que no se leyó ninguna contraseña.
- Cada prueba usa su propio esquema `archie_test_unified_*` y lo borra. Al terminar
  quedaban solo `public` y los esquemas de TimescaleDB.
- El clúster quedó detenido, no borrado.

## 4. Corrida real sobre Bonsai: interrumpida por la GPU

El dueño autorizó la GPU («si, autorizo gpu») y lo confirmó en el chat de esta sesión.

Configuración:
- `wanda-llm.sh gpu`: llama.cpp build 10777, contexto de 64k, 4 slots, todas las capas
  en la RTX 3070, `--reasoning-format auto`.
- Prueba `harness2_completes_on_a_real_llama_cpp_server_under_reply_schemas`, con
  razonamiento activado, 4096 tokens de salida, 120 s por etapa y 300 s por corrida.

Resultado: falló a los 35,4 s.

| Etapa | Prompt | Generado | Tiempo |
|---|---:|---:|---:|
| 1 solver base, bajo `artifact` | 347 | 2112 | 17,7 s |
| 2 proponente, bajo `harness` | 3774 | 678 | 7,8 s |
| 3 proponente | — | — | cortada a los 8,6 s |

- La etapa 1 se aceptó: el worker solo pidió la 2 después de leerla. La 2 no detuvo la
  corrida; desde fuera no se distingue una propuesta válida de un `ProposerError`.
- A las 00:56:51 llama-server cayó con `CUDA error: unknown error` en
  `ggml_backend_cuda_synchronize` (`cudaStreamSynchronize`).
- Desde entonces, `nvidia-smi` en Windows dice: "Unable to determine the device handle
  for GPU0: 0000:0A:00.0: GPU is lost. Reboot the system to recover this GPU". En la
  WSL, NVML no inicializa.
- La misma tarjeta registró eventos `nvlddmkm` 13, 14 y 153 el 2026-10-17. Hoy no quedó
  ningún evento del driver.
- llama-server quedó detenido (0 procesos). No se reinició nada ni se corrió
  `wsl --shutdown`: el reinicio lo decide el dueño.

No se afirma nada sobre la calidad del modelo ni sobre la tasa de éxito con esquema: son
2 etapas de una sola corrida.

## 5. Pendiente

- Repetir la corrida real de 8 etapas sobre Bonsai después de reiniciar Windows, con el
  comando de la prueba (ver `docs/HARNESS-INTEGRATION.md`).
- Proveedores en la nube con `strict: true` (OpenAI, Nebius, NIM): no probados, porque no
  se leyeron claves.
- Mi implementación local anterior, superada por PR 49, queda en el stash `7937cec` y
  como `sleepy-cerf-schema-local-2026-10-22.patch` en la carpeta de notas compartidas.
  La rama `claude/sleepy-cerf-01c183` sigue en `1d7f1c2`.
