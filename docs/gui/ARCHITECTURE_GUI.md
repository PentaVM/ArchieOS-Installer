# Arquitectura de la GUI profesional (propuesta)

Estado: **PROPUESTA para revisión humana.** Ningún ADR de este documento está aceptado. Lo que depende de hardware o software que esta sesión no pudo ejecutar (Linux, Wayland, NVIDIA, Quickshell) se marca `A VERIFICAR`.

## 1. Principios heredados del repositorio

1. **Los modelos proponen, Cerebra decide, Policy autoriza, el Stage ejecuta** (`ARCHIE.md`). La GUI es un cliente sin privilegios: muestra estado observado y solicita; nunca ejecuta ni autoaprueba.
2. **La pantalla no puede dañar el plano de control** (RFC-006): proceso y cuenta separados, proyección de sólo lectura, sin secretos, `source: live|fixture` siempre visible. Lo no medido nunca se parece a lo inactivo.
3. **El Orbe está terminado y no se toca**: `display/native` (render y lógica), `archie-orb`, `archie-displayd /api/orb`.
4. **Una persona siempre gana el control físico** y la GUI no puede devolvérselo al agente.

## 2. Vista de capas

```text
┌─────────────────────────── sesión del usuario (Wayland) ───────────────────────────┐
│  Quickshell (QML)  — Dock · Launcher · Control Center · Activity · Settings · Vault │  ← NUEVO (presentación)
│     │ JSON por líneas sobre socket Unix local, esquema versionado                    │
│  archie-shell-bridge (Rust, sin privilegios)  — adaptador tipado                     │  ← NUEVO (mínimo)
└──────────┬──────────────────────┬───────────────────────────┬──────────────────────┘
           │ lectura              │ lectura                   │ operaciones de archivos
  archie-displayd :7666     socket de control         archie-vaultd (nuevo, ADR-GUI-003)
  (existente, solo lectura) archied (solo lectura)             │ solicita a Policy
           │                      │                            ▼
           └──────────► archied ◄─┴──────────── Policy: Resource::Filesystem + Action + Scope::OneOperation
                          │
                  Cerebra → Policy → Stage → recibo → audit
Separado e intacto:  Observatorio (EGLFS o ventana) con Orbe · Stage (compositor Qt) · archie-voice
```

Reglas de la capa nueva:

- QML no abre archivos, no ejecuta procesos y no escribe en sockets de autoridad. Sólo habla con `archie-shell-bridge`.
- `archie-shell-bridge` expone capacidades con `ui_required = YES|INDIRECT` del registro, cada una con esquema, versión, timeout, cancelación e idempotency key. Sin canal para algo ⇒ el control no se dibuja (o se dibuja deshabilitado con motivo real).
- Si `archied` o el bridge caen, cada superficie muestra el estado real (`offline`, `degraded`, `stale` con edad) y reconecta; nunca conserva un "conectado" falso.

## 3. Flujo de una acción con efecto (patrón único)

```text
usuario → Quickshell: intención (p. ej. mover archivo)
       → bridge: solicitud tipada {correlation_id, idempotency_key, operación, parámetros}
       → servicio con autoridad (vaultd / archied): pide lease a Policy
              ├ denegada → error tipado → la GUI muestra el motivo real
              ├ requiere confirmación → evento "escalada" → diálogo de autorización (cancelable, sin consentimiento implícito)
              └ concedida → ejecuta → progreso → recibo → audit
       ← la GUI refleja progreso y resultado SOLO de eventos reales
```

Hoy el protocolo de `archied` es de sólo lectura. Los pasos "solicitud" y "respuesta a escalada" **no tienen contrato**. Son los bloqueos P0 del backlog (`GUI-POLICY-001`, `GUI-POLICY-002`, `GUI-EXEC-001`).

## 4. ADR

### ADR-GUI-001 — Shell: Quickshell/QML (Qt 6)

- **Decisión propuesta:** Quickshell para dock, launcher, paneles y overlays; componentes de presentación en Qt Quick puro, con una capa fina específica de Quickshell, para poder probar los componentes sin Quickshell.
- **Versión candidata:** v0.3.2 (2026-10-08), licencia LGPL-3.0 (dependencia dinámica). Fijar la versión al integrar; no usar snippets sin comprobar contra esa versión.
- **Razón:** el repositorio ya es Qt 6 (Observatorio, Stage, asistente). Mismo toolkit, mismas bibliotecas, un único runtime QML.
- **Alternativa descartada como base:** Noctalia v5 (MIT): es Wayland/GLES nativo sin Qt, no se integra con QML existente. Sigue siendo referencia de alcance UX.
- **A verificar:** requisitos exactos de protocolos Wayland (capas, foreign-toplevel, etc.), comportamiento con NVIDIA propietario.

### ADR-GUI-002 — Compositor y modos de sesión

Hechos verificados en el repositorio:

- **Modo principal actual:** Observatorio sobre EGLFS/KMS, sin compositor ni gestor de ventanas (`distro/display/archie-display.service`, RFC-006). Las apps se abren dentro del **Stage**.
- **El Stage es un compositor propio** (Qt WaylandCompositor). Su `Main.qml` registra `XdgShell` y gestores de entrada de texto; no hay protocolo de capas (búsqueda de `layer-shell` en el árbol: 0 resultados).
- **sway ya se usa** en `display/session/archie-session.sh` (headless) con el Stage en el workspace 1 y el Observatorio en el workspace 2 (`Mod4+1/2`). Es precedente verificado de coexistencia Orbe↔ventanas por cambio de workspace.

Decisión propuesta: **dos modos, no un escritorio permanente.**

| Modo | Pantalla | Compositor | Contenido |
|---|---|---|---|
| **Archie** (principal, sin cambios) | Observatorio EGLFS o ventana | ninguno / el actual | Orbe + voz + apps en el Stage |
| **Workspace** (opcional) | sesión Wayland interactiva | compositor wlroots-class (candidato inicial: **sway**, ya presente en scripts de sesión; evaluar **Hyprland** BSD-3 v0.56.2 para blur/animación) | Quickshell + Vault + ventanas; Stage y Observatorio como ventanas hijas |

Matriz de evaluación (sólo se rellena lo verificado; el resto se mide en Linux antes de aceptar):

| Criterio | sway | Hyprland | COSMIC comp | KWin |
|---|---|---|---|---|
| Ya usado por el repo | **sí** (`archie-session.sh`) | no | no | no |
| Licencia | (no consultada) | BSD-3-Clause | GPL-3.0 | (no consultada) |
| Capas para Quickshell | A VERIFICAR | A VERIFICAR | A VERIFICAR | A VERIFICAR |
| NVIDIA propietario (RTX 3070) | A VERIFICAR | A VERIFICAR | A VERIFICAR | A VERIFICAR |
| Blur/animación nativa | no esperada | sí (A VERIFICAR) | A VERIFICAR | A VERIFICAR |
| XWayland, multi-monitor, escala fraccional | A VERIFICAR | A VERIFICAR | A VERIFICAR | A VERIFICAR |

Condición de aceptación: prueba en Linux nativo con NVIDIA de (a) arranque del shell, (b) Stage y Observatorio como ventanas hijas sin pérdida de frame callbacks (el repo documenta que una ventana tapada deja de recibir frames), (c) impacto en latencia de IRIS.

Requiere aprobación humana porque contradice parcialmente RFC-006 §3 ("un entorno de escritorio: rechazado"). Argumento: el modo Workspace es opcional, separado y no sustituye el modo Archie.

### ADR-GUI-003 — Vault (gestor de archivos): opción A

- **Decisión propuesta: opción A** — GUI QML propia con modelo de interacción de COSMIC Files, sobre un backend nuevo `archie-vaultd` (Rust) que realiza el trabajo con el sistema de archivos.
- **Por qué no B (lanzar `cosmic-files`):** es una aplicación libcosmic/iced, no un control embebible en QML; no hereda el tema; su licencia GPL-3.0 obliga a cumplir GPLv3 en la distribución; y sus operaciones no pasarían por Policy, rompiendo "la GUI nunca se salta la autoridad". Sirve, a lo sumo, como visor "abrir con" para la persona.
- **Backend:** `archie-vaultd` recibe operaciones tipadas, solicita a Policy un lease `Resource::Filesystem{path}` con `Action::{Read,Write,Delete}` y `Scope::OneOperation`, confina rutas con `archie_workstation::host::workspace_path` (no se reimplementa en QML), registra en el audit, informa progreso/cancelación, y usa papelera según la especificación de freedesktop. Descomprime con los límites de `archie_toolpack::unpack` (`Limits`). Las miniaturas se decodifican en un proceso aislado. El crate **no existe**; crearlo es la Fase C.
- **Diseño visual:** ver `SCREEN_SPECS.md`; paridad en `GUI_FUNCTION_BACKLOG.md` (`GUI-VAULT-001..024`), derivada de `pop-os/cosmic-files@ac5cef76fa60`.
- **Primeros contenidos reales posibles sin backend nuevo:** la carpeta de entregas `~/Archie/<proyecto>/` que ya genera `archie-desk` (con `index.html` y `stages.json`).

### ADR-GUI-004 — Nombre "Vault"

`archie-vault` ya es el almacén cifrado de secretos y `/vault` es su ruta. Propuesta: en la GUI, **Vault = gestor de archivos**; el almacén de secretos se muestra como **Sealed** (estado, auditoría, recibos de divulgación; nunca contenido). El crate no se renombra (no se cambian contratos existentes). Si se prefiere otro nombre para el gestor, cambia sólo la capa de presentación.

### ADR-GUI-005 — Adaptador al IPC real

- **Decisión propuesta:** un binario Rust mínimo `archie-shell-bridge`, sin privilegios, que (1) lee de `archie-displayd` (HTTP loopback tipado y de sólo lectura), (2) lee del socket de control de `archied` cuando la cuenta de sesión pertenezca al grupo permitido, y (3) enruta las operaciones con efecto hacia los servicios con autoridad. Quickshell habla con él por un socket Unix local con JSON por líneas versionado.
- **Por qué no `Process {}` desde QML:** sería ejecución de comandos desde la interfaz, prohibida por el handoff.
- **Por qué no dar el grupo `archie` a la sesión:** RFC-006 lo rechazó expresamente para el renderizador; el bridge es el único componente con ese grupo y su superficie es mínima.
- **Dependencias abiertas:** contrato para responder escaladas de Policy y para pausa de emergencia (no existen).
- **Estado en la Fase C (implementado):** `core/crates/archie-shell-bridge`, **sólo lectura**. Difiere de la propuesta en un punto: en vez de un socket Unix que escucha, el bridge se ejecuta como **proceso hijo con argumentos fijos** (`archie-shell-bridge watch`) y escribe una trama JSON por línea en su salida estándar. Así no abre ningún puerto ni socket para terceros. El socket queda para cuando haya operaciones con efecto. Sólo lee de `archie-displayd` (rechaza direcciones no loopback), propaga `source: live|fixture` y declara el enlace con siete estados explícitos (`waiting`, `live`, `fixture`, `daemon_unreachable`, `displayd_offline`, `invalid`, `bridge_silent`). La capa Quickshell que lo lanza (`display/shell/quickshell/BridgeProcess.qml`) **no se ha ejecutado**.
- **Ampliación (segunda rebanada):** además lee, sin escribir nunca, dos fuentes de archivo ya existentes: el inventario de `archie-lan watch` (`--lan-file`, formato `archie-lan-inventory-v1`) y los `status.json` de los trabajos largos de `archie-bridges` (`--workspace` / `--workspaces-root`). No inicia escaneos ni cancela trabajos: cancelar escribe un archivo `cancel` y queda fuera hasta que haya una vía autorizada. El campo libre `latest` de cada trabajo no se reenvía.


### ADR-GUI-006 — Coexistencia con el Orbe

El Orbe permanece en su proceso. La transición Orbe↔Workspace es un **cambio de workspace del compositor** (precedente: `archie-session.sh show stage|orb`), no una reconstrucción. El shell nunca embebe ni redibuja el Orbe. Los puntos de entrada/salida son: chip de estado del Orbe en la barra (lee `/api/orb`), atajo para volver al Orbe, y `DisplayControl` (7668) para cambiar la vista del Observatorio. El conflicto del puerto 7668 queda como decisión humana (§AUDIT 4).

### ADR-GUI-007 — Rendimiento con IA en paralelo

Perfiles `Full / Balanced / Quiet / Reduced Motion` ligados a `GET /api/kernel` (`display_budget`, que el repo ya calcula para que "una máquina saturada devuelva visiblemente la GPU"). El shell consume ese mismo presupuesto en vez de inventar uno. Los presupuestos de FPS/VRAM se fijan tras medir una línea base; ninguna cifra de este documento es un resultado.

## 5. Decisiones que requieren revisión humana antes del merge

1. Aceptar dos modos de sesión pese a RFC-006 (ADR-GUI-002).
2. Crear `archie-vaultd` y `archie-shell-bridge` (nuevos crates con superficie de autoridad).
3. Nombre "Vault" para el gestor frente a `archie-vault` (ADR-GUI-004).
4. Contratos nuevos en `archied`: responder escaladas, pausa de emergencia, listado de tareas. Tocan el plano de control; el handoff prohíbe cambiarlo "para acomodar widgets", por lo que cada uno necesita su RFC.
5. Conflicto de puerto 7668.
6. Re-auditoría tras fusionar #28, #29, #30.
