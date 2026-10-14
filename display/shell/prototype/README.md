# Prototipo de la shell profesional · diseño Archie Mac (Fase B)

Qt Quick puro, **sin Quickshell**, datos sintéticos rotulados ("DATOS DE DISEÑO"). No habla con
`archied`, `archie-displayd` ni el sistema de archivos. Sustituye nada de `display/native`
(Orbe, Observatorio) ni de `display/stage`.

Ejecutar (Qt 6.6+, Linux o Windows; en Windows sin `qml` basta PySide6, ver `display/visor/README.md` para la esfera):

```sh
qml display/shell/prototype/Main.qml
```

Teclas: `Ctrl+K` paleta · `C` centro de control · `A` actividad · `S` Sentry · `G` lista/cuadrícula ·
`M` movimiento reducido · `O` superficies opacas · `B` desenfoque del compositor (cristal) · `Esc` cerrar.

### Ventanas y sistema básico (14 oct)

Las cinco apps del dock (Vault, Forge, Sentry, Stage, Ajustes) son ventanas de verdad, gobernadas por `Wm.qml`
(estado real; el contenido de cada ventana sigue siendo de ejemplo):

- **Mover:** arrastrar la barra de título (queda siempre alcanzable). **Subir al frente:** cualquier clic en ella.
- **Luces:** roja cierra, amarilla minimiza al dock, verde amplía (otra vez, restaura). Grises si la ventana no está al frente.
- **Dock:** un punto bajo lo abierto (atenuado si está minimizado). Clic: abre, restaura, sube o, si ya está al frente, minimiza.
  Clic derecho: menú de la app (traer al frente, minimizar, zoom, cerrar); en el Orb, mostrar el escritorio, minimizar o cerrar todas.
- **Teclas:** `Ctrl+W` cierra la ventana del frente, `Ctrl+Mayús+W` las cierra todas, `Ctrl+M` minimiza, `Ctrl+Mayús+M` las minimiza todas,
  `D` muestra el escritorio, `F` `S` `,` abren Forge, Sentry y Ajustes (otra vez, con la ventana al frente, la cierran).
  `W` abre el selector de ventanas (lista las reales): `Tab` / `Mayús+Tab` eligen, `↵` abre.
- **Menú Ventana:** minimizar, zoom, cerrar, minimizar/mostrar/cerrar todas, mostrar el escritorio y las cinco apps.
- **Reloj real** en la barra y en el bloqueo. **Silencio** (Centro de control) es real pero solo de esta shell: oculta los avisos.
  El bloqueo no autentica: solo tiene «Volver al escritorio». Ajustes > Apariencia reúne colores, movimiento, opacidad y cristal.
- **Esfera de fondo** (Ajustes > Visor gráfico): nodos visibles, color (Original, Holograma frío, Vivo), polvo de luz, brillo, actividad, giro.

Superficies de ejemplo de la segunda entrega (todas "Diseño · datos de ejemplo", sin datos reales):
`W` Alt-Tab · `N` centro de notificaciones · `T` avisos + diálogo de autorización · `F` Forge ·
`I` inspector IRIS/Cortex · `,` Ajustes · `L` bloqueo/arranque · `Mayús+L` asistente de instalación ·
`X` selector de espacio (o el chip «Dominio») · `/` búsqueda. `V` recorre los estados
(normal, vacío, error, offline, degradado, desactualizado) de esas superficies, `.` activa el anillo
de foco, `H` deja el escritorio vacío, `P` cambia el perfil de movimiento (Full / Balanced / Quiet).

Visor gráfico (fondo del escritorio): la esfera de Archie Lumenfield 0449 sobre el grafo guardado del
contenido del sistema, servida por `../../visor` (ver su README). `Ctrl+Espacio` libera la cámara para
navegar en 3D y `Esc` la vuelve a fijar. Ajustes > Visor gráfico (perfil, brillo, actividad, giro) y
Ajustes > Apariencia (colores vívidos u originales). Sin el visor en marcha se ve el fondo de siempre.

Aspecto: diseño Archie Mac de Claude Design; ver `docs/gui/DESIGN_ARCHIE_MAC.md` (qué se aplicó, desviaciones
y correcciones a la verdad). Fuentes: `../fonts` (OFL). Referencia: `../design/archie-mac-handoff`.
Las capturas de `docs/gui/screenshots/` se generaron con Qt 6.12 (PySide6, API gráfica de software,
plataforma `offscreen`), a 1920×1080 y 2560×1440.
El script de captura no se versiona (ArchieOS no incluye Python). Los mismos resultados con
`qml` en Linux no están verificados.

Pruebas: `BridgeModelTest.qml` (modelo de datos), `SurfacesTest.qml` (superficies de ejemplo y perfiles de movimiento) y `WmTest.qml` (gestor de ventanas, luces y dock); sin QtTest, con resultados en `passes` / `failures` / `log`.

**No probado:** Quickshell (`PanelWindow`, capas), blur del compositor, NVIDIA, Wayland,
lectores de pantalla, `qmllint`.

## Qué está conectado a datos reales (Fase C, primera rebanada)

El enlace y los medidores del centro de control salen de `archie-shell-bridge`
(`core/crates/archie-shell-bridge`), no de `Synthetic.qml`:

- `BridgeModel.qml`: recibe las líneas JSON (`ingest`) y deriva el estado del enlace.
  `BridgeModelTest.qml` lo prueba (15 comprobaciones, sin QtTest).
- `LinkChip.qml` (barra superior y centro de control) y `ResourceGauges.qml` (PSI y térmica).
- `frames/real-*.jsonl`: tramas **reales** impresas por el binario del bridge contra el
  `archie-displayd` real (sin `archied`) y contra un puerto sin servidor.
  `frames/synthetic-live-with-psi.jsonl`: trama **sintética** con la forma de los tipos reales,
  para dibujar el estado `live` con PSI y térmica, que en Windows no existen.
- Reproducir una trama en el prototipo: definir la propiedad de contexto `replayFile` (URL de un
  `.jsonl`) y `QML_XHR_ALLOW_FILE_READ=1`. El shell real alimenta el modelo con
  `../quickshell/BridgeProcess.qml` (no probado).

Sigue siendo sintético y rotulado "DISEÑO": el Vault, la actividad, la paleta, el dock y los
interruptores de Wi-Fi/Bluetooth/Silencio/Privacidad (ahora "sin servicio conectado").

## Segunda rebanada: Sentry (red local) y trabajos largos

- `SentryPanel.qml` (tecla `S` o icono de Sentry en el dock): dispositivos del inventario que
  escribe `archie-lan watch`, leído por el bridge. Sin botones de acción. Estados distintos para
  "no configurado", "el escáner aún no ha corrido", "sin permiso", "no válido", "vacío" y
  "desactualizado".
- `ActivityCenter.qml`: trabajos largos reales (`status.json` de `archie-bridges`), con barra sólo
  si el trabajo declaró un total y "sin novedades hace N" en vez de afirmar que está colgado.
  **Sin botón de cancelar.** La autorización y la auditoría siguen como ejemplo de DISEÑO.
- Barra superior: el contador "◔ N" son los trabajos en curso reales ("—" si no hay espacios).
- Tramas nuevas: `real-lan-missing.jsonl` (real: el archivo no existe aquí),
  `synthetic-lan-inventory.jsonl`, `synthetic-lan-stale.jsonl` y `synthetic-jobs.jsonl`: salida
  **real del binario** del bridge sobre archivos de entrada **sintéticos** (direcciones de
  documentación 192.0.2.x, nombres "(ejemplo)").
