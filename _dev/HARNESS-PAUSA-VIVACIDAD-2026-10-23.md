# Harness, fase 1: pausa de emergencia en el demonio y vivacidad de modelos locales — 2026-10-23

Rama `claude/unified-0.1` (base `5b1864d`), en `PentaVM/ArchieOS-Nvidia`. Cierra los
puntos 11 y 13 del frente harness de `Desktop/ArchieOS/AUDITORIA-0.1-2026-10-23.md`
(M4 y M5 del plan consolidado, salvo el botón de `ControlCenter.qml:72`, que es de la
GUI de Claude/Xavi y no se tocó).

## 1. Pausa de emergencia (`archie ai pause | resume | status`)

Antes: `runtime.rs:471` creaba el `PolicyEngine` con `emergency_pause: false` fijo, y
`ai_control.rs:75` clonaba una copia por petición. `set_emergency_pause` existía, pero
nada del demonio lo llamaba.

Ahora:
- `Runtime` guarda un solo motor vivo, `Arc<std::sync::RwLock<PolicyEngine>>`.
  `Runtime::policy()` devuelve una copia para una decisión (Sentry y la sesión de
  operador). `Runtime::live_policy()` y `Runtime::ai_authority()` dan el motor mismo, y
  todo `AiInfer`/`AiRecall`/`AiHarness` admite, despacha y confirma contra él.
- Verbos nuevos `AiPause`, `AiResume` y `AiStatus`. Solo pasan por el socket del operador
  con par del kernel (`is_ai()`); el despachador público los rechaza. Los admite
  `RuntimeAuthority::admit_profile_owner`: solo el dueño de `etc/inference.toml`, sin
  mirar `enabled` ni la pausa, para que el dueño siempre pueda reanudar. Respuesta:
  `AiPolicy { paused, recorded }`.
- Pausar fija el motor primero y después registra `state_dir/policy/emergency-pause`
  (`core/crates/archie-core/src/emergency.rs`) y la auditoría
  `policy.emergency_pause`. Reanudar quita el registro y audita
  `policy.emergency_resume` antes de soltar el motor; si algo falla, el nodo sigue en
  pausa. El arranque lee el registro: un nodo pausado arranca pausado. Un registro que no
  se puede inspeccionar cuenta como pausa.
- `archie ai ask|recall|harness2 --standalone` (sin demonio) respeta el registro.
- Es la pausa de Core: también niega cualquier otro arriendo mutante (Sentry) mientras
  dura. Está documentado en `docs/HARNESS-INTEGRATION.md` §"Emergency pause".

## 2. Vivacidad de modelos locales

Antes: con Bonsai marcado `ready` y llama-server caído, la voz y `archie-ask` elegían
igual a Bonsai (`Disclosure::Kept`), fallaban y no pasaban a la ruta velada.

Ahora (`core/crates/archie-model-runtime/src/liveness.rs`):
- Antes de rutear se pide `GET /health` al servidor de cada modelo llama.cpp local
  usable: 500 ms de plazo, sin proxy ni redirecciones, respuesta guardada 5 s en un
  registro por proceso. Conexión rechazada, plazo vencido o 5xx (llama-server responde
  503 mientras carga) = offline para el router hasta que responda. 404 cuenta como vivo.
  No se sondean modelos cloud ni otros proveedores, y la sonda no lleva nada de la
  pregunta.
- `Routed` rutea sobre el registro con esos modelos en `Offline`. `ask` y `ask_private`
  refrescan primero. Si en `ask_private` falla un modelo local que respondió a la sonda,
  se olvida su respuesta, se vuelve a sondear y se decide de nuevo: la pregunta puede ir
  velada dentro del plazo que quede.
- `UnifiedRuntime` (AiInfer y Harness²) refresca antes de `candidates`. Una corrida fijada
  a un modelo caído se rechaza ("pinned model is offline or outside this request's
  scope"), nunca se mueve a otro modelo.
- La voz (`archie-voice/src/nemotron.rs`) refresca antes de `disclose_chat`.

## 3. Pruebas (WSL `ArchieOS-Operator-RTX3070-Test`, `cg.sh`, objetivo compartido)

Nuevas:
- `archie-core`: `emergency::tests` (2), `ai_control.rs`
  `emergency_pause_verbs_are_operator_bound_and_status_does_not_mutate` y, con PG,
  `owner_pause_stops_admitted_and_inflight_dispatch_until_the_owner_resumes`. Esta prueba
  cubre cinco casos:
  1. Un permiso admitido antes de la pausa no se despacha durante ella, y sí después de
     reanudar.
  2. Una pausa durante la respuesta del modelo impide confirmarla y mostrarla.
  3. En pausa no llega nada al modelo.
  4. Tras reanudar se completa una petición, y hay 4 entradas de auditoría.
  5. Un nodo arrancado sobre un registro de pausa arranca pausado.
- `archie-workstation`: `the_owner_is_known_while_paused_or_disabled_and_no_one_else_is`,
  y con PG `a_ready_local_model_whose_server_is_down_is_passed_over_not_asked`. Usa un
  puerto cerrado y un servidor que responde 503: ninguno recibe la pregunta, contesta el
  siguiente candidato, y la corrida fijada se rechaza.
- `archie-model-runtime`: `liveness::tests` (5: puerto cerrado, servidor vivo con una
  sola sonda por TTL, 503/404, vencimiento del TTL, solo llama.cpp local) y
  `tests/liveness_routing.rs` (3: Bonsai caído → velado o rechazo; Bonsai vivo → se queda
  en la máquina; servidor que cae tras su sonda → velado).
- `archie-cli`: `pause_commands_parse_and_a_disagreeing_record_is_spoken_aloud`.
- Los fixtures llama.cpp de `unified_runtime.rs` responden ahora `/health`
  (`accept_call`).

Fallan con el comportamiento anterior. Se reintrodujo cada causa a mano, se corrió la
prueba y después se restauró el código:

| Causa reintroducida | Prueba | Resultado |
|---|---|---|
| Copia del motor en `ai_control` | prueba PG de pausa | panic en el paso 2: el resultado se mostró pese a la pausa |
| Sin `refresh_liveness` en `UnifiedRuntime` | prueba PG de modelo caído | `asked` 4 ≠ 0 |
| Sin nueva decisión en `ask_private` | `a_server_that_stops_…` | probó `local.bonsai-2-9b`, no la nube |

Logs: `/var/tmp/archie-logs/h1-ff-{core,unified,routed}.log`.

Validación final, todo en `/var/tmp/archie-logs/`:

| Paso | Resultado | Log |
|---|---|---|
| `cargo fmt --all -- --check` | limpio | `h1-post-fmt.log` |
| `clippy --workspace --all-targets -D warnings` | rc=0 | `h1-final-clippy.log` |
| `test --workspace --no-fail-fast` | 2878 pasan, 0 fallan, 152 ignoradas | `h1-final-test.log` |
| PG `archie-core` `ai_control` + `harness_control` (`--ignored`) | 3/3 y 3/3 | `h1-post-pg-core.log`, `h1-post2-pg-core.log` |
| PG `archie-workstation` `unified_runtime` (`--ignored`, sin la prueba de llama real) | 12/12 | `h1-pg-unified.log`, `h1-post2-pg-unified.log` |

En `h1-post-pg-unified.log`, una corrida falló 11/12. La prueba que falló fue
`original_policy_lease_clamps_transport_and_expired_result_stays_failed`, en su cota de
reloj de 140 ms (`unified_runtime.rs:527`). Ese camino usa un proveedor OpenAI-compatible,
que no se sondea. Sola pasó 5 de 5 veces (`h1-flaky-1..5.log`), y la suite completa volvió
a pasar 12/12. Es una prueba sensible al tiempo con carga, no un efecto de este cambio.
La prueba `harness2_completes_on_a_real_llama_cpp_server_under_reply_schemas` se omitió:
necesita GPU y Bonsai.

## 4. Qué no prueba esto

- Nada corrió contra Bonsai ni contra un llama-server real. La GPU no se usó. Los
  servidores son fixtures HTTP en loopback, y la nube es un puerto HTTPS cerrado, así
  que ninguna pregunta salió de la máquina.
- `archied`/`archie` instalados en la distro siguen siendo los del 2026-10-15. No se
  reinstalaron ni se probó el demonio real por systemd.
- El botón de pausa de `ControlCenter.qml:72` sigue deshabilitado: la GUI es de
  Claude/Xavi. El backend ya tiene el verbo (`ai_pause` en el socket del operador).
- `CoreAuthority` de Sentry guarda una copia del motor tomada al aprobar. Una pausa entre
  la aprobación y el despacho de esa misma petición no la detiene; las siguientes, sí.
- Solo el dueño del perfil pausa. Ni root ni los `operator_uids` de Sentry pueden hacerlo
  por el verbo; root puede crear el archivo de registro y reiniciar.
- La sonda solo cubre llama.cpp. Un Ollama/vLLM local caído sigue fallando en la llamada,
  y `UnifiedRuntime` pasa al siguiente candidato como antes.
