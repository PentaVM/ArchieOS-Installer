# Especificación de pantallas

Estado: **Fase B.** Cobertura visual real: 5 de las 15 superficies del handoff (marcadas ▣). Las demás sólo tienen especificación textual. Capturas reproducibles en `docs/gui/screenshots/` (1080p y 1440p, Qt 6.12 software, datos sintéticos rotulados). Todas las pantallas deben soportar: estado vacío, error, `offline` (daemon caído), `degraded` y `stale` con edad del dato; teclado completo; foco visible; salida segura (`Esc`); Reduced Motion.

Mapa de capturas: `01-frost`, `02-workspace`, `03-signature`, `04-synthesis` (lista), `05-synthesis-grid`, `06-synthesis-palette`, `07-synthesis-control`, `08-synthesis-activity`, y `*-1440p`.

| # | Superficie | Cobertura | Backlog | Notas de diseño |
|---|---|---|---|---|
| 1 | Session / Workspace | ▣ parcial (Vault + Stage como ventanas hijas) | `GUI-STAGE-001` | ventanas gestionadas por el compositor; el shell sólo dibuja chrome propio |
| 2 | Dock inteligente | ▣ | `GUI-DISPLAY-003` | punto de ejecución sólo para procesos verificados (`AppCatalog` los cuenta por `/proc`); lanzar = acción de la persona; marcar "Archie puede operarla" desde la política del Stage |
| 3 | Launcher universal | ▣ (paleta) | `GUI-SEARCH-001` | catálogo = registro de capacidades filtrado por permisos; nunca busca en la web al teclear |
| 4 | Command Palette | ▣ | idem | acciones Archie reales; las `BLOCKED` aparecen deshabilitadas con motivo |
| 5 | Control Center | ▣ | `GUI-CORE-011`, `GUI-DISPLAY-002`, `GUI-POLICY-002` | sólo servicios reales; lo no medido = regla discontinua; pausa de emergencia deshabilitada con motivo |
| 6 | Notificaciones / Actividad | ▣ | `GUI-BRIDGE-002`, `GUI-EXEC-001`, `GUI-POLICY-001`, `GUI-CORE-012` | progreso real + auditoría real; autorización con Rechazar como foco por defecto |
| 7 | **Vault** | ▣ | `GUI-VAULT-001..024` | ver abajo |
| 8 | Alt-Tab / overview | texto | `GUI-STAGE-001` | agrupa por app y dominio |
| 9 | Búsqueda y relaciones | texto | — | **sin World Graph en el repositorio**: no se dibuja relación alguna |
| 10 | Forge | texto | `GUI-BRIDGE-001`, `GUI-FORGE-001` | catálogo de bridges (real) vs módulo Forge (sin worker): se muestran por separado |
| 11 | Sentry Center | texto | `GUI-SENTRY-001..` | inventario LAN real (`archie-lan`); consentimiento y cámara con indicador derivado de `CameraAuthority` |
| 12 | Inspector IRIS/Cortex | texto | `GUI-IRIS-*`, `GUI-CORTEX-001` | sólo Debug Mode; sin pensamiento privado ni telemetría inventada |
| 13 | Ajustes | texto | `GUI-CORE-018/019`, `GUI-MODEL-001` | proveedores: sólo el **nombre** de la variable de entorno; nunca el valor |
| 14 | Onboarding / bloqueo | texto | ninguno (`CAP-INSTALL-001` es `NO_GUI_NEEDED`) | el asistente existe (`display/installer`); no se duplica un servicio de autenticación |
| 15 | Coexistencia con el Orbe | ▣ parcial (chip + selector) | `GUI-DISPLAY-004` | cambio de workspace del compositor; el Orbe no se redibuja |

## Vault (superficie 7)

**Anatomía (verificada en las capturas):** barra de título con vista lista/cuadrícula; pestañas con "+"; migas de pan; barra lateral (ubicaciones con insignia de dominio personal/trabajo, fila **Sealed** de sólo estado); área de contenido en lista (nombre, tamaño, modificado, contexto) o cuadrícula; cola de operaciones inferior con progreso, permiso concedido, Cancelar y Deshacer (sólo si hay reversión).

**Extensiones propias de ArchieOS:** carpeta fija *Entregas* (`~/Archie/<proyecto>/stages/…`, producida por `archie-desk`), insignia "Archie lo tocó" (a derivar del audit real), insignia de dominio, Sealed sin contenido.

**Flujos:**
- *Copiar a Entregas:* selección → pegar → solicitud tipada → Policy lease de una operación (lectura origen + escritura destino) → progreso real → recibo. Denegado → mensaje con el motivo real. Cancelación a mitad: lo copiado se conserva y se informa.
- *Borrar:* a la papelera con confirmación; **borrado definitivo** exige escalada propia y texto explícito.
- *Abrir:* lo hace la persona (no el agente), con la misma regla que `AppCatalog::launch`.
- *Backend caído / volumen desmontado / lease expirado:* estado `degraded` con acción de reintento; nunca una lista fantasma.

**Teclado (propuesto, por validar):** `Ctrl+T` pestaña, `Ctrl+L` ruta, `Ctrl+F` buscar, `F2` renombrar, `Supr` papelera, `Mayús+Supr` definitivo, `Alt+←/→` historial, `Ctrl+1/2` lista/cuadrícula.

**Pendiente de diseño:** vista previa/galería, panel dividido, menú contextual, diálogo selector de archivos, propiedades; diálogos de renombrado y conflicto de nombres; miniaturas.

## Responsive y accesibilidad

Verificado: proporcionalidad en 1080p y 1440p. **No verificado:** lector de pantalla (AT-SPI), escala fraccional, multi-monitor, táctil. Jerarquía de accesibilidad prevista: cada fila de archivo = elemento con rol, nombre, estado de selección y contexto; `Accessible.*` aún no está añadido al prototipo.

## Actualización Fase C: qué muestra datos reales

Capturas `C-*` en `screenshots/`: `C-real-daemon-unreachable` (trama **real**: `archie-displayd` real, sin `archied`; telemetría "no medido" con la razón real del kernel), `C-real-displayd-offline` (trama **real**: nada escucha) y `C-synthetic-live-with-psi` (trama **sintética** con la forma de los tipos reales: la única forma de ver `live` con PSI/térmica en este equipo). Datos reales en el prototipo: chip de enlace (barra superior y centro de control), resumen de sistema (sólo si `archied` respondió) y medidores de presión/temperatura. Las chips de **cámara/micro** y **dominio** pasaron de afirmar un estado a decir "sin lectura", porque su fuente (`CameraAuthority`, `PillarContext`) aún no está conectada.

## Actualización Fase C, segunda rebanada

- **Sentry Center (superficie 11)** pasa de texto a render: capturas `S-real-lan-missing` (trama real: el inventario no existe en este equipo), `S-synthetic-lan-inventory` y `S-synthetic-lan-stale` (bridge real sobre un inventario sintético). Pasivo, sin acciones.
- **Actividad (superficie 6)**: los trabajos largos son reales; capturas `A-synthetic-jobs` (bridge real sobre un espacio de trabajo sintético) y `A-real-daemon-unreachable` (sin espacios configurados). Cancelar no se ofrece.
- Cobertura visual: 6 de 15 superficies renderizadas; con datos reales en parte: barra superior, centro de control, Sentry y actividad.

## Actualización: diseño Archie Mac

Las capturas de `screenshots/` y su nomenclatura se sustituyeron por las del diseño Archie Mac (ver `DESIGN_ARCHIE_MAC.md`); los nombres `0x-synthesis`, `C-*`, `S-*` y `A-*` de arriba corresponden al diseño anterior y están en el historial de git. Las posiciones y tamaños de ventanas y paneles son ahora las del diseño.

## Actualización: segunda entrega de Claude Design

Renderizadas ahora, rotuladas «Diseño · datos de ejemplo»: Alt-Tab (8), notificaciones y diálogo de autorización (6), Forge (10), inspector IRIS/Cortex (12), Ajustes (13), bloqueo/arranque/instalador (14), selector de espacio y hueco del Orbe (15) y búsqueda (9). Con las anteriores, **las 15 superficies tienen render**; ninguna de las nuevas tiene datos reales detrás. Detalle, estados y decisiones abiertas en `DESIGN_ARCHIE_MAC.md` §8. Capturas `screenshots/11` a `28`.
