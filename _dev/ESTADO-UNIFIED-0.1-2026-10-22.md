# Estado de la integración unificada alpha 0.1 — 2026-10-22

Rama `claude/unified-0.1` en `PentaVM/ArchieOS-Nvidia` (worktree
`C:\Users\Penta\Desktop\ArchieOS-harness2`), sobre `fc4e509`. Escrito por la sesión de
Claude que terminó la integración. Todo lo medido se corrió en la WSL
`ArchieOS-Operator-RTX3070-Test` (usuario `archie`), con Bonsai en la RTX 3070. Lo que no
se pudo probar está dicho como tal.

## 1. Commits de esta sesión

| Commit | Qué |
|---|---|
| `e2342d5` | Proveedor `LlamaCpp` (siempre `Local`, sin credencial). Manifiestos de Bonsai en `models/local/`. Interruptor de razonamiento también en streaming. Valor mal escrito de `thinking` rechazado. Códigos de error estables en los recibos. |
| `8a96328` | Un modelo fijado se pide en el rol que declara su manifiesto. Presupuesto de razonamiento de llama.cpp: la mitad de la salida, con mensaje de cierre. |
| `083be60` | Contrato de respuesta del worker de Harness² que un modelo local real puede cumplir (ver §3). |
| `2691c83` | `ARCHIE.md` nombra los modelos de Bonsai (prueba de contrato MCP). |
| `1c84b2e` | Arreglos de la revisión independiente (§5). |
| (último) | `docs/HARNESS-INTEGRATION.md` y este estado. |

Autor de todos: PentaVM. No hay merge a `main`.

## 2. Harness² sobre Bonsai: medido

Configuración:
- `llama-server` build 10777 (`e3de71522`) en `127.0.0.1:8081`, alias `bonsai`;
  Ternary-Bonsai-2-9B PTQ1_0, `--ctx-size 65536 --parallel 4`, todo en GPU.
- Modelo fijado `local.bonsai-2-9b` (razonamiento activado).
- Perfil del operador: 4096 tokens de salida, 120 s por llamada, 32 KiB de prompt.
- Raíz `/var/tmp/archie-h2-bonsai`, esquema `archie_test_h2_bonsai`.
- Tarea pública sintética: decidir `ask_owner`, `draft` o `nothing` ante una invitación a
  una feria que el dueño no ha respondido (el mismo espacio de acciones de la prueba del
  correo del 21).

Velocidad de Bonsai en la GPU, medida directo contra llama-server: 85–122 tokens/s de
generación.

**Corrida completa `bonsai-think-20261022o`: `completed`.**

| Etapa | Entrada | Salida | Tiempo |
|---|---:|---:|---:|
| 1 solver base | 783 | 2130 | 18,7 s |
| 2 proponente | 4245 | 2280 | 23,2 s |
| 3 proponente | 4107 | 2267 | 23,3 s |
| 4 solver | 783 | 2131 | 21,1 s |
| 5 solver | 783 | 2121 | 21,0 s |
| 6 proponente (composición) | 4200 | 947 | 11,4 s |
| 7 solver | 783 | 2142 | 20,9 s |
| 8 reflexión | 3707 | 875 | 10,4 s |

- **Libro de upstream:** `R=4`, `P=3` (esperados 4 y 3), `k=1`, modo `seq`, rollouts
  `base1, inv1, dom1, c1`, sesiones `inv1, dom1, c1`, 8 llamadas.
- **Uso:** 19 391 tokens de entrada y 14 893 de salida. Costo `null`: es local y no se
  inventa un cero.
- **Tiempo:** 150,7 s de pared, 150,1 s de ellos en llamadas al modelo.
- **Recibos:** 8, todos `completed`, con resultado sellado en el Vault. IDs del
  `73d7c244…` al `1127ccb3…`; están en la tabla `inference_receipts` del esquema.
- **Selección:** trivial (`c1`).
- **Promoción:** `baseline_retained`, `reason_code: no_trusted_assessor`,
  `improvement_verified: false`. No se escribió nada en la biblioteca.
- **Contenido**, recuperado del Vault con `archie ai recall`:
  - los 4 rollouts del solver eligieron `ask_owner`, con preguntas como "Do you want to
    attend the International Renewable Trade Fair in April 2027?";
  - las 3 propuestas devolvieron la semilla sin cambios;
  - la reflexión anotó "no observed effect" y aun así escribió una lección ("confirm owner
    intent before proceeding with any reply action"). La promoción la rechazó por defecto.
- **Reanudación:** repetir la misma identidad devolvió `resumed` en 73 ms, con 0 recibos
  nuevos.
- **No se afirma ninguna mejora del modelo:** nadie la evaluó, y las propuestas no
  cambiaron nada.

**Confirmación sobre el código final (`1c84b2e`), corrida `bonsai-final-20261022p`:**
- Con los manifiestos tal como se distribuyen (`offline`), Harness² se rechazó en 117 ms,
  sin llamar al modelo.
- Con las copias instaladas marcadas `ready` (el paso de operador documentado), la corrida
  se completó: 8 etapas, `R=4`/`P=3`, 19 621 tokens de entrada y 16 149 de salida, 169,3 s
  de pared (168,6 s en el modelo, entre 12,0 y 24,1 s por etapa), 8 recibos completados,
  `baseline_retained` / `no_trusted_assessor`.
- Las dos últimas corridas, con todos los arreglos, se completaron. No es una tasa de
  éxito: son 2 muestras.

**Bonsai sin razonamiento**, una sola muestra con `archie ai ask` y
`local.bonsai-2-9b-fast`: ante la misma decisión respondió "nothing", en 1,5 s. Con
razonamiento, los 4 rollouts de la corrida `o` eligieron `ask_owner`. Es coherente con la
prueba del correo del 21, pero no es una medición de calidad.

## 3. Qué hizo falta para llegar ahí

Antes de esa corrida hubo 12 corridas por el CLI y 2 con un padre de diagnóstico. Cada
una se detuvo en un defecto distinto, que se arregló con una regresión que falló primero:

1. **Rol del modelo fijado.** El servicio unificado pedía todo como `CognitiveResponse`;
   `local.bonsai-2-9b` solo declara `cognition.reasoning`, así que no había candidato.
2. **Semilla contradictoria.** `systemprompt.md` decía "plain text" y el worker exige
   JSON. Bonsai deliberó sobre el conflicto hasta agotar 4096 tokens (`finish=length`, sin
   respuesta).
3. **Razonamiento sin tope.** En el proponente, Bonsai razonó más de 4096 tokens. Ahora
   llama.cpp recibe `thinking_budget_tokens` (la mitad de la salida) y
   `reasoning_budget_message`, ambos por petición; se verificó que esta build los respeta.
4. **Claves con prefijo `harness/`**, copiadas del listado de contexto. La instrucción
   ahora nombra las 4 claves exactas.
5. **`trajectory` y `notes` como listas** de cadenas, y una vez como lista de objetos. Se
   leen como texto: son registro de lo observado, nunca se ejecutan ni se adoptan.
6. **Respuesta dentro de un bloque ```json.** Se quita una única cerca alrededor de toda
   la respuesta del modelo; las tramas del protocolo siguen estrictas.
7. **`guardrails.md` con un salto de línea final.** Una copia que solo difiere en
   espacios se reemplaza por los bytes exactos de la base. Cualquier otro cambio aborta.
8. **Propuesta ilegible** (JSON roto). Ahora es el `ProposerError` de upstream: la etapa
   conserva la base, la llamada se cobra y la corrida sigue.
9. **Reflexión en JSON.** La instrucción ahora pide las etiquetas de upstream, "never
   JSON".
10. **`NO_ENTRY`.** Upstream lo pide cuando no hay lección que transfiera. Antes era la
    falla `missing_experience`; ahora la corrida se completa con `baseline_retained` /
    `no_lesson`, sin pedido de promoción y sin entrada en la biblioteca. El puente Rust
    acepta solo ese caso.

Lo que no cambió: una edición fuera de la superficie o de `guardrails.md` sigue
abortando (la prueba de ediciones maliciosas pasa), la promoción sigue denegada por
defecto, y `Secret`, `Restricted` y la localidad no se tocaron.

**Pendiente recomendado:** decodificación restringida por esquema JSON en las etapas JSON.
Una sonda directa mostró que llama.cpp respeta `response_format: json_schema` con el
razonamiento activado. Requiere llevar el esquema de la etapa hasta `ChatRequest`, que se
construye en varios crates; no se hizo esta noche. Mientras tanto, la salida de Bonsai
varía entre corridas y una corrida puede fallar por una forma nueva.

## 4. Codex oficial desde Rust: bloqueo externo

Probado en orden después del reinicio de la VM (01:09):
1. **Control** `cmd.exe /d /c exit 0` desde la WSL: exit 1 a los 10 s, firma
   `UtilAcceptVsock:250: accept4 failed 110`. `WSL_INTEROP` no existe en el proceso.
2. **Lectura de cuenta desde la WSL**
   (`probe_codex_host.py rust`): `EOFError` antes de inicializar, la misma firma.
3. **Turno sintético con reanudación:** no se intentó, porque falla en la misma frontera
   antes de cualquier protocolo.

Contraste: en Windows nativo, la misma sonda inicializa y lee la cuenta (`chatgpt_auth`,
identidad estable). El bloqueo está solo en el cruce WSL→Windows. No se hizo
`wsl --shutdown`: requiere permiso del dueño, y mata a Bonsai, a Wanda en :8793 y al
clúster de prueba. La otra salida es un toolchain de Rust en Windows, que tampoco existe
hoy.

## 5. Validación completa y revisión independiente

Entorno de compilación: directorio existente `/var/tmp/archie-pr-integration-target`, 2
trabajos, sin debuginfo, `CUDA_VISIBLE_DEVICES` vacío. Logs en `/var/tmp/archie-logs/claude/`.

**Pasada sobre `083be60`** (antes de la revisión):

| Paso | Resultado |
|---|---|
| `cargo fmt --all -- --check` | limpio |
| `cargo clippy --workspace --all-targets -- -D warnings` | limpio (91 s) |
| `cargo test --workspace` | 2847 pasan, 1 falla, 147 ignoradas. La falla era el contrato MCP: `ARCHIE.md` no nombraba los modelos de Bonsai. Corregido en `2691c83`. |
| PostgreSQL ignoradas, `--test-threads=1`, clúster desechable en el puerto 55439 | `archie-postgres` 17, `archie-state` 56, `archie-core` 13 (`ai_control`, `harness_control`, `control_roundtrip`, `persistent_node`, `cognitive_context`, `cortex_projection`), `archie-workstation` 12 (`harness2_bridge` con upstream real, `unified_runtime`). Todas pasan. |
| Python del worker de Harness² (CPython 3.13.16) | 24/24 en esa pasada; 25/25 al final |
| Esquemas creados | solo `archie_test_*` (comprobado antes/después) |

Excluidas a propósito, por la regla de "solo esquemas `archie_test_*`":
- `full_timescale_restore_…` de `archie-state`, que crea bases de datos;
- las pruebas de PostgreSQL de `archie-sentry`, que usan esquemas `archie_sentry_test_*`;
- `sentry_operator` y `sentry` de `archie-core`, que necesitan terminal o un fixture de Sentry.

`archie-memory` no tiene pruebas de PostgreSQL propias: las de memoria viven en
`archie-postgres` y `archie-state`.

**Revisión independiente** (subagente nuevo, solo lectura, enfocado en las interacciones
entre fusiones). Corregido, con regresiones que fallaron primero (`1c84b2e`):
- **P1.** Los manifiestos de Bonsai como `cold` hacían que los modelos propios ganaran
  toda divulgación, en todo grado (voz, `archie-ask`, Arena), sin respaldo, en imágenes sin
  llama-server. Ahora se distribuyen `offline`; el nodo que sirve Bonsai marca sus copias
  `ready`. Con `cold` fallaban 9 pruebas de divulgación del árbol distribuido.
- **P1.** Un proveedor Ollama/vLLM en loopback marcaba como `Local` también los modelos
  `*-cloud` / `:cloud` que Ollama reenvía a ollama.com. Ahora son `Cloud` con techo `Public`.
- **P2.** Un resultado `Restricted` en un foco abortaba toda petición de menor grado en
  ese foco. La memoria propia del foco por encima del grado se omite (nunca se divulga);
  la evidencia citada explícitamente sigue fallando cerrada.
- **P2.** Cualquier reflexión ilegible se reportaba `no_lesson`. Solo `NO_ENTRY` explícito
  completa sin lección.
- **P2.** La prueba de contrato MCP (`2691c83`).

Registrado como pendiente, sin corregir:
- **P2, voz por `--endpoint` / `ARCHIE_VOICE_ENDPOINT`** (`archie-voice.rs:1443`). Construye
  un descriptor `NvidiaNim`, así que `archie-talk` contra Bonsai en :8081 no envía el
  interruptor de razonamiento y queda con el valor por defecto de la plantilla (razonando).
  Es terreno de la sesión de voz.
- **P2, Harness² sella todo como `Public`.** `HarnessRunRequest` no tiene grado. Ya está
  documentado ("nunca texto privado en una tarea"). Arreglo de fondo: un grado en la
  petición, que cambia el contrato de Core.
- **P3, pausa de emergencia muerta en el daemon.** `ai_control.rs` clona un
  `PolicyEngine` cuyo `emergency_pause` está fijo en `false`, y no hay setter. Hoy solo
  `enabled = false` en el perfil detiene una corrida.
- **P3, el Vault de inferencia crece sin límite.** Cada resultado reescribe
  `entries.json`; al tope de 64 MiB toda inferencia fallaría sin recibo.
- **P3, admisión de IA por "dueño de `inference.toml`"**, distinta de los
  `operator_uids` de Sentry.
- **Sin respaldo cuando un modelo propio falla.** Si el nodo marca Bonsai `ready` y
  llama-server cae, la voz y `archie-ask` no vuelven a la ruta velada de la nube: falta una
  disponibilidad viva (sondeo o marca `offline` al fallar).

La revisión dio por limpios el guard del Vault tras `fc4e509`, la superficie de control
(IA solo en el socket del operador), las reglas de ruteo (`Secret` rechazado en toda
entrada; `Restricted` nunca a Nebius) y el `Cargo.lock`. No revisó: internos de IRIS, el
LNCM y el entrenamiento de Wanda, los adaptadores de Sentry, SQL, el instalador, el display
ni los scripts de la WSL.

### 5.1 Pasada final sobre `91e362e`

| Paso | Resultado |
|---|---|
| `cargo fmt --all -- --check` | limpio |
| `cargo clippy --workspace --all-targets -- -D warnings` | limpio |
| `cargo test --workspace` | 2850 pasan, 0 fallan, 148 ignoradas |
| PostgreSQL ignoradas (`--test-threads=1`) | `archie-postgres` 17, `archie-state` 56, `archie-core` 13, `archie-workstation` 13; 0 fallas |
| Python del worker de Harness² | 25/25 |

Este commit solo cambia este documento.

## 6. Qué no se probó

- Ningún turno real de Codex administrado desde Rust (§4).
- Ninguna API en la nube (Anthropic, OpenAI, Nebius): no se leyó ni se buscó ninguna clave.
- `archied` como daemon con Harness²: la corrida medida fue `--standalone`. La ruta de
  Core (`AiHarness`) está cubierta por `harness_control` con PostgreSQL real y un servidor
  HTTP sintético, no con Bonsai.
- La segunda tarea que reusa la biblioteca: con la promoción denegada por defecto no hay
  entrada que reusar. El reuso está probado con el padre determinista de Python.
- Latencia de voz con Bonsai, calidad del modelo y despliegue en hardware final.
