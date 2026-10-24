# Unificación: Wanda por Core, voz sobre llama.cpp y un turno real en Nebius — 2026-10-24

Rama `claude/unified-0.1` (base `29ae567`), en `PentaVM/ArchieOS-Nvidia`. Cierra B4 y la
parte de B5 que se puede cerrar de `Desktop/ArchieOS/AUDITORIA-0.1-2026-10-23.md`
(Harness #3, #4 y #10; Rest G13).

## 1. Wanda pregunta por Core (`AiInfer`)

Antes: `archie-wanda/src/runtime.rs:360` armaba su propio `Routed` y llamaba al modelo.
No había permiso de Policy ni recibo, y la pausa de emergencia no la detenía.

Ahora, con un bloque `[core]` en la configuración de Wanda (`src/via_core.rs`):
- Cada consulta es un `ControlRequest::AiInfer` en el socket del operador de `archied`,
  fijado (`ApiPinned`) al modelo que Cerebra admitió, con grado `Restricted`,
  `recall_memory = false` y el `INSTRUCTION` de Wanda delante de su cápsula.
- Core la admite con `etc/inference.toml`, la despacha con un permiso del motor vivo de
  Policy, el router vuelve a filtrar grado y localidad, y la respuesta queda sellada en
  el Vault de Core. `/v1/query` devuelve el `receipt`.
- Solo cuenta como respuesta un recibo `completed` del mismo modelo fijado, con texto.
- Sin Core en marcha: `core_unavailable`, y Wanda no consulta al modelo por fuera.
  Core rechaza (pausa, perfil, dominio): `core_refused`. Plazo vencido: no se repite.
- Sin bloque `[core]` se mantiene la llamada directa, sin recibo (diagnóstico y pruebas
  sin base de datos). Documentado en `docs/WANDA-CONTEXT.md` §"Razonamiento por Core".
- El experto LNCM (estado 1) sigue en el proceso de Wanda: no es una llamada a un modelo
  del router.

## 2. Voz: Bonsai como modelo llama.cpp

Antes: `archie-voice --endpoint` registraba el modelo local como `NvidiaNim`
(`archie-voice.rs:1446`). Con eso la sonda `/health` no lo cubría (solo mira llama.cpp)
y no se enviaba el interruptor de razonamiento: Bonsai pensaba antes de cada respuesta.

Ahora:
- `--endpoint` registra un descriptor `LlamaCpp` (`local_voice`), contexto 16 384 (el
  slot de `archie-bonsai.service`), con `thinking` fijado por `--thinking off|on` o
  `ARCHIE_VOICE_THINKING`. Por defecto `off`. Un valor distinto se rechaza al construir
  el endpoint.
- `WandaVoiceAccess` acepta `LlamaCpp` en loopback, con las mismas condiciones que antes.
- El modelo de grounding local (`--ground-endpoint`) sigue siendo un NIM.

**Lo que no cambia, a propósito:** los turnos de la voz no pasan por `AiInfer`. Un turno
de voz es streaming con tools; `AiInfer` devuelve una respuesta sellada y no ofrece
tools. Pasar la voz por Core exige un `AiInfer` con streaming y tools. No se hizo. Lo que
la voz hace sigue bajo Policy y Stage, y su tool `wanda_context` llega a Wanda, que ahora
pasa por Core. Queda escrito en `docs/HARNESS-INTEGRATION.md` §"Who uses the single path".

## 3. Turno real en Nebius Token Factory

Primer turno real de Archie en la nube. Modelo `nebius.nemotron-3-super`
(`nvidia/nemotron-3-super-120b-a12b`), grado `public`, tarea sintética pública
(planificar la entrega de un póster de la VM al PC del dueño). Raíz de Archie
`/var/tmp/archie-h2-bonsai` (PostgreSQL en :55439, esquema `archie_test_h2_bonsai`).
La clave vino de `Desktop/clavenebiusapi.txt` (autorizado por el dueño para esta
sesión). Solo estuvo en el entorno del proceso que llamó, y ningún archivo de salida la
contiene (comprobado con `grep -F` sobre cada directorio).

| Corrida | Ruta | Recibo | Estado | Tokens entrada/salida | Tiempo |
|---|---|---|---|---:|---:|
| 1 | `archie ai ask --standalone` | `19653ef0-1fbc-4db9-95d2-78ac31e82fe9` | completed | 177 / 409 | 4,2 s |
| 2 | `archied` → socket del operador → `AiInfer` | `fe09f186-ac9c-4f75-92e0-b05cd8ab3595` | completed | 392 / 433 | 3,5 s |

- En la corrida 2, Core trajo como evidencia la respuesta sellada de la corrida 1 (misma
  Focus `nebius-demo`): `admitted_evidence_refs` = `vault:personal/owner/inference/19653ef0…`.
- `archied` se compiló de este árbol, arrancó con la clave en su entorno, sirvió el turno
  y se detuvo con SIGTERM 4 s después. `archie ai status` antes del turno:
  `{"paused":false,"recorded":false}`.
- Evidencia: `_dev/verification/integration/nebius-20261024/` (prompt, los dos JSON,
  `archied.log`, sha256 de los binarios).
- El texto de Nemotron es una propuesta sin verificar. Mezcla papeles (dice que el
  "Model" termina el render y que Cerebra empaqueta el archivo). No es una medición de
  calidad.

## 4. Pruebas (WSL `ArchieOS-Operator-RTX3070-Test`, objetivo compartido)

Nuevas:
- `archie-wanda/tests/runtime.rs`:
  - `with_core_configured_and_no_core_running_nothing_reaches_the_model`: 0 llamadas al
    modelo, estado `core_unavailable`.
  - con PG: `core_answers_wanda_with_a_receipt_and_its_pause_stops_her`. Un `Runtime` de
    Core real en el socket del operador. Wanda recibe 1 propuesta con recibo y el modelo
    recibe 1 llamada. Con `AiPause` la consulta siguiente da `core_refused` y el modelo
    no recibe nada. `AiResume` vuelve a dejarlo en marcha.
- `archie-voice` (bin): `the_local_voice_is_bonsai_on_llama_cpp_with_its_thinking_pinned`.
- `archie-voice/src/wanda.rs`: el caso Bonsai en
  `the_routed_voice_offers_wanda_only_if_every_restricted_choice_is_loopback`.

| Paso | Resultado | Log (`/var/tmp/archie-logs/`) |
|---|---|---|
| `cargo check` wanda + voz (`wanda`) | rc=0 | `u1-check.log` |
| `cargo test -p archie-wanda` | 27 pasan, 0 fallan, 3 ignoradas | `u1-test-wanda.log` |
| `cargo test -p archie-voice --features wanda --lib --bin` | 68 pasan | `u1-test-voice.log` |
| PG `archie-wanda --test runtime --ignored` | 1/1 | `u2-pg-wanda.log` |
| `cargo fmt --all -- --check` | limpio | `u3-fmt.log` |
| `clippy --locked -D warnings` wanda, voz, cli, archied | rc=0 | `u3-clippy.log` |
| `clippy --locked -D warnings` voz `speech,live,rvc` | rc=0 | `u3-clippy-live.log` |
| `cargo test -p archie-voice --features speech,live,rvc` | 123 pasan, 1 ignorada | `u3-test-voice-live.log` |

`clippy` con `speech,live` sin `rvc` falla por `rvc_key`, `rvc_index_rate` y
`ort_library` sin leer (`archie-voice.rs:753`). Es anterior a este cambio: el PR 48 se
validó con `speech,live,rvc`.

No se corrió la suite completa del workspace. Los cambios tocan solo `archie-wanda` y
`archie-voice`, y `archie-cli` y `archied` compilan con clippy estricto.

## 5. Qué no prueba esto

- Wanda por Core se probó con un servidor HTTP de prueba, no con Bonsai. No había
  llama-server corriendo, y la GPU no se usó.
- La voz con el descriptor nuevo no se probó contra un llama-server real ni con
  micrófono.
- La configuración de la demo de Wanda (`~/wanda-demo/wanda.toml`) todavía no tiene
  `[core]`. Los `archied`/`archie` instalados en la distro siguen siendo los del
  2026-10-15. Los nuevos están solo en el objetivo compartido.
- Hermes, Codex y los turnos en Anthropic y OpenAI no se tocaron.
