# Auditoría de la codebase para la GUI profesional

Estado: **Fase A (auditoría)**. Todo lo que figura aquí se verificó leyendo el árbol o ejecutando un comando; lo que no se pudo verificar está marcado `UNVERIFIED` o `UNKNOWN`. Este documento no afirma que ninguna interfaz nueva exista.

## 1. Procedencia

| Dato | Valor |
|---|---|
| Repositorio | `PentaVM/ArchieOS-Nvidia` (privado; permiso del usuario autenticado: `WRITE`) |
| Rama base | `main` @ `0bd64058c7e9a56ada66be386e2366bf566330d9` |
| Rama de trabajo | `feat/professional-gui-quickshell-vault` |
| Fecha de la auditoría | 2026-10-11 |
| Archivos versionados | 1070 |
| Tamaño aproximado | 45 crates Rust (149 845 líneas `.rs`), 46 137 líneas C/C++, 3 109 líneas QML, 2 026 funciones `#[test]` |
| Repositorios complementarios | `PentaVM/ArchieOS` (privado, visible, **no auditado**: es otro árbol, según `_dev/HANDOFF.md`). `PentaVM/archie-iris` (citado en `README.md`): **HTTP 404 con los permisos actuales**, no se pudo inspeccionar. |
| PRs abiertos al momento de auditar | [#28](https://github.com/PentaVM/ArchieOS-Nvidia/pull/28) `fix/lan-limits` (+8 820/−56), [#29](https://github.com/PentaVM/ArchieOS-Nvidia/pull/29) `feat/sentry-port` (+7 226/−56), [#30](https://github.com/PentaVM/ArchieOS-Nvidia/pull/30) `feat/sentry-complete` (+8 820/−56). Añaden un crate `archie-sentry` y cambian `archie-core`, `archie-lan`, `archie-workstation`, `archie-state`, `archie-cli`. **Esta auditoría no los incluye**: deben re-auditarse al fusionarse. |

Árbol de trabajo: no había modificaciones locales previas (clon nuevo). Los documentos de `docs/gui/` son lo único sin versionar.

## 2. Mapa de componentes

| Capa | Ruta | Lenguaje | Responsabilidad real |
|---|---|---|---|
| Plano de control | `core/crates/archied`, `archie-core`, `archie-policy`, `archie-scheduler`, `archie-execution`, `archie-executord`, `archie-task-runner`, `archie-fabric`, `archie-state`, `archie-postgres`, `archie-registry`, `archie-observability`, `archie-types` | Rust | Identidad, política, leases, planificación, ejecución sandboxed, auditoría, estado en PostgreSQL/Timescale |
| Cognición | `archie-cerebra`, `archie-router`, `archie-model-runtime`, `archie-memory`, `archie-iris`, `archie-iris-bridge`, `archie-sight`, `archie-a11y`, `archie-reflex`, `archie-rem`; `cortex/` (C++23) | Rust / C++ | Bucle cognitivo, enrutado de modelos con descalificación por privacidad, memoria con procedencia, visión, REM nocturno |
| Manos | `archie-stage` + `display/stage` (compositor Wayland Qt), `archie-paint`, `archie-arena`, `archie-bridges` (10 plugins por app), `archie-cut`, `archie-toolpack`, `archie-mcp`, `archie-mcp-server`, `archie-desk` | Rust / C++ / QML | Entrada sellada y con lease, automatización por aplicación, entrega de resultados |
| Voz | `archie-voice`, `archie-speech` | Rust | Conversación voz a voz, suelo conversacional, herramientas propuestas |
| Mundo | `sentry/` (C++), `archie-lan`, `archie-worldd`, `archie-geofabric`, `display/osm`, `capture-agents/` (Kotlin, JS) | C++ / Rust | Observaciones con procedencia, inventario LAN, grafo de personas, ingesta de captura |
| Presentación | `display/native` (Observatorio: Orb, Forge, World, Qt Quick 3D), `display/visual` (modelo sin dependencias de GPU), `archie-displayd`, `archie-orb`, `display/installer` (asistente Qt) | C++ / QML / Rust | Proyección de sólo lectura; Orb en vivo |
| Distribución | `distro/archiso`, `distro/wsl`, `distro/display`, `distro/database` | shell / systemd | ISO de Arch, rootfs WSL2, servicios |
| Herramientas de desarrollo | `tools/engineering-dashboard` (Next.js), `tools/owner-companion` (Node, WebAuthn) | TS / JS | Fuera del producto; el `owner-companion` participa en el instalador |
| Módulos declarativos | `modules/**/*.toml` (35 manifiestos) | TOML | Declaran qué existe y por qué carga o no |

## 3. Grafo de autoridad (verificado en `ARCHIE.md`, `docs/STAGE.md`, `docs/CEREBRA.md`, `core/crates/archie-stage`)

```text
entrada (voz / texto / IRIS)
  → interpretación: un modelo PROPONE (planner, Nemotron u otro; IRIS propone acciones "proposal_only")
  → Cerebra DECIDE si algo debe ocurrir y qué puede revelarse        (archie-cerebra::pass::propose_stage_action)
  → Policy AUTORIZA con Lease de una operación                       (archie-policy::PolicyEngine::decide)
  → Stage EJECUTA: StageGate → Sealer (HMAC-SHA256, secuencia, TTL ≤ 10 s)
        → compositor C++ verifica MAC antes de parsear, geometría, límites y keymap
  → recibo: executed | cancelled | código de rechazo → a todos los clientes
  → auditoría (archie-observability::audit::AuditEvent) y estado persistente
```

Reglas que la GUI no puede romper (todas verificadas en `docs/STAGE.md`): una persona siempre gana el control físico; el texto tecleado nunca se muestra ni se registra; un comando está ligado a la superficie y revisión de geometría para la que se preparó; una captura necesita su propio lease; los recibos no sobreafirman.

## 4. Interfaces y puertos (IPC real)

| Canal | Dirección / formato | Escritura | Fuente |
|---|---|---|---|
| Socket de control de `archied` | `/run/archie/archied.sock` (Windows dev: TCP loopback). JSON por líneas. 13 peticiones: `status topology modules providers peers devices device_operations cortex_trajectories models boot metrics audit ping` | **Ninguna.** `ControlRequest::is_mutating()` devuelve `false` siempre | `archie-core/src/control.rs:1328-1380` |
| `archie-displayd` | HTTP loopback `127.0.0.1:7666`: `/api/snapshot /api/world /api/kernel /api/orb /api/orb/lumen /api/geofabric /healthz`. Cada instantánea lleva `source: live|fixture` | Ninguna | `archie-displayd/src/lib.rs:194-290` |
| `archie-worldd` | `127.0.0.1:7667`, adaptador USGS | Ninguna | `distro/display/archie-worldd.service` |
| Control del Observatorio | TCP `127.0.0.1:7668`, una línea JSON con escena/perfil/overlay permitidos | Sólo vista | `display/native/src/DisplayControl.cpp:57` |
| Stage | `$XDG_RUNTIME_DIR/archie-stage/control.sock`, comandos sellados con HMAC; eventos `surface unmapped focus damage authority receipt snapshot` | Sí, sólo comandos sellados | `archie-stage/src/protocol.rs` |
| Cortex | socket Unix, binario versión 1, sin comandos ni credenciales | No | `cortex/include/archie/cortex/ipc.hpp` |
| `archied`↔`archie-executord` | socket privilegiado: `Start Stop Preempt Resume Rebudget Inventory Drain` | Sí, sólo `archied` | `archie-execution/src/protocol.rs` |
| `archie-desk` | `127.0.0.1:7447`, entregas con HMAC y ventana de 5 minutos | Escribe `~/Archie/…` | `archie-desk/src/lib.rs` |
| `archie-voice live` | página en `127.0.0.1:8770` (WebSocket micro/altavoz/cámara) | n/a | `archie-voice/src/bin/archie-voice.rs` |
| MCP | stdio, herramientas filtradas por clearance | según herramienta | `archie-mcp-server/src/*.rs` |
| `archie-lan` | escribe un archivo JSON (`watch --out FILE`) | Archivo | `archie-lan/src/main.rs` |

**Conflicto de puertos:** `127.0.0.1:7668` lo usan a la vez `DisplayControl` (Observatorio), `tools/owner-companion/server.mjs` y el asistente de instalación (`display/installer/Wizard.cpp`). El instalador corre antes del Observatorio, pero un nodo con ambos activos fallaría al enlazar. No se modifica en este PR (es contrato existente); queda como decisión de revisión humana.

**Consecuencia para la GUI:** el protocolo de control es de sólo lectura por diseño, y no hay ningún canal tipado para (a) responder una escalada de Policy, (b) pausar la autoridad de emergencia, (c) listar tareas, ni (d) operar archivos. Cualquier botón con efecto necesita un contrato nuevo y autorizado; ver `ARCHITECTURE_GUI.md` y los ítems `BLOCKED` del backlog.

## 5. Estado por subsistema

Vocabulario del handoff aplicado al código leído. Detalle y símbolos exactos: `CAPABILITY_REGISTRY.csv` (114 capacidades canónicas) y `CAPABILITY_SYMBOL_INDEX.csv` (3 841 símbolos públicos, todos asignados a un componente cubierto).

| Subsistema | Estado | Evidencia / matiz |
|---|---|---|
| Core / `archied` | IMPLEMENTED | 13 vistas de sólo lectura, arranque, hotplug, ejecución |
| Policy | IMPLEMENTED | `set_emergency_pause` existe pero **no tiene llamadores ni IPC** (`grep`) |
| Executor | IMPLEMENTED | `archie-executord`, Landlock/cgroups; no hay vista de tareas en el protocolo de control |
| Scheduler | IMPLEMENTED | interno |
| Cerebra | IMPLEMENTED | `propose_stage_action`, pilares personal/trabajo |
| Stage (compositor + política + sello) | IMPLEMENTED | `display/stage`, pruebas de gate/seal/operate |
| IRIS | PARTIAL | el modelo corre como servicio externo `archie-iris`; `README.md` dice "Not yet: an IRIS inference runtime inside ArchieOS" |
| Voz | IMPLEMENTED | `converse`, `live`, herramientas; ASR requiere NeMo-Speech.cpp |
| Sentry (C++) | IMPLEMENTED (biblioteca + CLI) | `camera_worker`, `gstreamer_source`, `onnx_detector` requieren GStreamer/ONNX: **PARTIAL** en la práctica |
| LAN | IMPLEMENTED | `archie-lan`, módulo `sentry.lan` de tipo `process` |
| Cortex (C++) | IMPLEMENTED | demonio sin privilegios, IPC binario v1 |
| Orbe | IMPLEMENTED | `archie-orb` + `display/native`; **no se toca** |
| Observatorio (Forge/World/Orb) | IMPLEMENTED | "Forge" aquí es un catálogo de apps (`AppCatalog`), no el módulo FORGE |
| Vault (almacén de secretos) | IMPLEMENTED | `archie-vault`: AES-256-GCM, `ContextGateway`, `Veil` |
| **Vault (gestor de archivos)** | **NO EXISTE** | ni crate, ni QML, ni protocolo |
| REM | IMPLEMENTED | `archie-rem` (`status/once/watch`) |
| Toolpack | IMPLEMENTED | catálogo firmado, instalación atómica, rollback |
| Bridges (10 apps) | IMPLEMENTED | `docs/bridges/INDEX.md` está desactualizado (ver `TEST_REPORT.md`) |
| Instalador | IMPLEMENTED | asistente Qt ya existe |
| Tablero Enterprise (`board_api`) | PARTIAL | el router sólo lo referencian pruebas; ningún binario lo monta |
| NOVA | STUB | `modules/core/nova.toml`: `worker.kind = "unimplemented"` |
| FORGE (módulo) | STUB | `modules/core/forge.toml`: `worker.kind = "unimplemented"` |
| WATCHER | **no es un servicio** | `SceneMode::Watcher` es la escena del mundo del Observatorio (etiqueta de UI "WORLD") |
| World Graph | NO EXISTE | cero coincidencias de `world_graph`/`World Graph` |
| Módulos declarativos | 31 de 35 `unimplemented` | sólo `core.archie` y `core.router` (`builtin`), `core.rem` y `sentry.lan` (`process`) tienen worker |

## 6. Conciliación con los nombres del handoff

| Nombre del handoff | Qué existe realmente |
|---|---|
| Core / `archied` | Existe. |
| Cortex | Existe en dos sentidos: demonio C++ `cortex/` (compilador de contexto acotado) y vista `CortexTrajectories`. |
| Cerebra / Policy | Existen y son crates separados. |
| **Verifier** | **No hay un componente único.** `modules/cognition/verifier.toml` es un manifiesto sin worker. "Verifier" en `archie-fabric`/`archie-stage` significa verificación de firma/MAC. La verificación de acciones ocurre en `StageGate`, en el compositor (`check_command`) y en `archie-sight` (comprueba que la ventana reaccionó). |
| **Executor** | Dos sentidos: `archie-executord` (supervisor de procesos) y el **Stage** (ejecuta acciones de entrada). El README resume "Stage executes". |
| IRIS | Parcial: ver §5. |
| SENTRY / NOVA / FORGE / WATCHER / World Graph | Ver §5. |
| Orbe | Terminado. Vive en el Observatorio (`display/native`), alimentado por `archie-orb` y `archie-displayd /api/orb`. |
| Vault | **Colisión de nombre**: `archie-vault` ya es el almacén cifrado de secretos y `/vault` es su ruta en WSL. El handoff usa "Vault" para el gestor de archivos. Ver ADR-GUI-004. |

## 7. Contradicciones y huecos documentales

1. **RFC-006 rechaza expresamente un entorno de escritorio** y define el Observatorio como cliente de sólo lectura sobre EGLFS sin compositor. El handoff pide un workspace profesional. Son modos distintos; se propone conservar ambos (ADR-GUI-002), pero requiere aprobación humana.
2. `README.md` dice que GitHub Actions está pausado por facturación y que los checks se ejecutan en local; `.github/workflows/ci.yml` existe y define jobs. No se verificó cuál refleja la realidad.
3. Hay dos documentos de arquitectura (`ARCHITECTURE.md` raíz y `docs/ARCHITECTURE.md`); no se comparó su coherencia.
4. Los manifiestos `unimplemented` explican que "el worker Python nunca existió y ArchieOS no incluye runtime de Python": la GUI debe mostrar `not_implemented`, no ocultar el módulo.
5. `docs/bridges/INDEX.md` no coincide con la salida de `archie-bridges index` (línea de instalación de GIMP en Ubuntu); el test `index::tests::the_committed_index_is_current` falla en `main`.
6. `_dev/HANDOFF.md` distingue dos árboles (`ArchieOS-Nvidia` y `ArchieOS`); qué capacidades viven en cuál lo explica el handoff compartido, no este PR.

## 8. Funciones sin forma segura de exhibirse en GUI (decisión: no se dibujan)

- Introducir un secreto en el almacén (`Vault::store`): campo de contraseña en QML = clase prohibida por las reglas del handoff. Requiere un prompt seguro del sistema.
- `archie enrol --owner-key HEX`: es una clave privada por `argv`.
- Pausa de emergencia y respuesta a escaladas de Policy: no hay IPC; dibujar el botón sería un botón falso.
- Cualquier cosa que alcance `archied`↔`archie-executord`, la clave del Stage o `PostgresStore`.
- Pensamiento privado de Cerebra/planner y rastros de `Veil` (la tabla de reversión nunca sale del nodo).

## 9. Compatibilidad de plataforma

- Producto: Arch Linux (ISO) y Arch en WSL2. El árbol soporta un endpoint TCP en Windows "sólo desarrollo".
- Este equipo de auditoría es Windows 11 **sin WSL, Qt, CMake, Ninja ni Quickshell**. `archie-arena` no compila en Windows (`tokio::net::UnixStream`). Por tanto **nada de la capa Qt/QML/Quickshell se ha compilado ni renderizado aquí**.
- Debug Mode: existe `display/session/archie-session.sh` (sway headless + wayvnc + noVNC, Stage en workspace 1 y Observatorio en workspace 2). Es la referencia para pruebas visuales en Linux.

## 10. Licencias (verificadas con `gh api` el 2026-10-11)

| Proyecto | Licencia | Última versión / commit consultado | Uso permitido en este repositorio (Apache-2.0) |
|---|---|---|---|
| ArchieOS-Nvidia | Apache-2.0 | `0bd6405` | — |
| Quickshell | LGPL-3.0 | v0.3.2 (2026-10-08), `4f508be500de` | Dependencia dinámica, sin copiar código |
| COSMIC Files | GPL-3.0 | `master` `ac5cef76fa60`, sin releases | **Referencia de comportamiento sólo; no copiar código.** Usarlo como app externa (opción B) obliga a cumplir GPLv3 en distribución |
| COSMIC comp | GPL-3.0 | `master` `4d14528f9108` | Como compositor externo (proceso separado) |
| Noctalia | MIT | v5.2.1; `4b4dedef3a92` | Referencia UX. **v5 es Wayland/GLES nativo sin Qt ni GTK**, no Quickshell (confirmado en su README) |
| DankMaterialShell | MIT | v1.6.3; `6640a943b30d` | Referencia; copiar requiere conservar el aviso MIT |
| Caelestia shell | GPL-3.0 | v2.5.0; `21a75375bb33` | **Referencia visual sólo; copiar código contaminaría Apache-2.0** |
| Hyprland | BSD-3-Clause | v0.56.2 | Compositor candidato |

**No hecho:** auditoría de licencias de las dependencias Cargo/npm del repositorio (`cargo deny`/`cargo license` no se ejecutaron). Pendiente antes de añadir dependencias nuevas.

## 11. Qué sigue sin verificarse

- Compilación y pruebas C++ (`ctest` de `cortex`, `sentry`, `display/visual`, `display/osm`): sin CMake aquí.
- Comportamiento real de Quickshell sobre NVIDIA/Wayland y su requisito exacto de protocolos del compositor (se documentará en el ADR con pruebas en Linux antes de aceptarlo).
- RTX 3070 8 GB: no hay hardware ni acceso a él desde esta sesión.
- Contenido de `PentaVM/archie-iris` y de `PentaVM/ArchieOS`.
