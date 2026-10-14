# Informe de pruebas

Estado: **línea base de la Fase A.** Este PR no contiene aún código de GUI, así que no hay pruebas de GUI que informar. Se registra qué se pudo y qué no se pudo ejecutar para que nadie confunda "no corrió" con "pasó".

## Entorno de la ejecución

| Elemento | Valor |
|---|---|
| SO | Windows 11 Pro 10.0.26200 |
| Rust | cargo/rustc 1.98.0 (el workspace declara `rust-version = 1.88`) |
| Node / Python | 24.19.0 / 3.13.14 (Python sólo para generar los CSV de auditoría; no entra al repositorio) |
| Ausentes | WSL, Qt, CMake, Ninja, qmllint, Quickshell, compositor Wayland, GPU NVIDIA accesible |
| Commit | `0bd64058c7e9` (`main`) |

## Línea base Rust (workspace, `--locked`, sin `archie-arena`)

Comando: `cargo test --workspace --exclude archie-arena --locked --no-fail-fast`

| Resultado | Cantidad |
|---|---|
| Suites ejecutadas | 200 |
| Pruebas pasadas | 1 900 |
| Pruebas ignoradas | 78 |
| **Pruebas falladas** | **1** |

**Fallo:** `archie-bridges` → `index::tests::the_committed_index_is_current` — "docs/bridges/INDEX.md is stale". Reproducido regenerando el índice con `archie-bridges index` y comparando: difiere **una línea** (`Install on Ubuntu` de GIMP: el código dice "GIMP 3 AppImage…", el archivo commiteado dice `flatpak install flathub org.gimp.GIMP`). Los archivos están en LF (`git ls-files --eol`: `i/lf w/lf`), así que **no es un artefacto de Windows**: es un defecto previo de `main`. No se corrige en este PR (fuera de alcance).

**No ejecutado:** `archie-arena` no compila en Windows (`unresolved import tokio::net::UnixStream`, `tokio::net::unix`). Un primer intento con el workspace completo se abortó por esto; el resultado de la tabla excluye ese crate. Se espera que compile en Linux.

## Pruebas C++ (cortex, sentry, display/visual, display/osm)

**No ejecutadas** — no hay CMake ni Ninja en este equipo. `ci.yml` define el job `native-contracts` que las ejecuta; no se verificó si CI corre (el `README.md` dice que GitHub Actions está pausado por facturación).

## Pruebas de GUI

Ninguna todavía. Cuando exista código se informará, por cada ítem `GUI-*`: prueba positiva, prueba negativa (permiso denegado / backend caído / cancelación), accesibilidad, movimiento reducido.

## Pendiente de medir (no hay números todavía)

- Percentiles de tiempo de fotograma p50/p95/p99, jank, RAM/VRAM, latencia entrada→pantalla aproximada y consumo en reposo, en 1080p y 1440p.
- Impacto sobre IRIS/inferencia en RTX 3070 8 GB (hardware no disponible en esta sesión).
- Wayland/NVIDIA nativo, XWayland, multi-monitor, escala fraccional.

## Riesgos registrados

1. La capa Qt/QML/Quickshell no se ha compilado ni renderizado en esta sesión; todo lo que entregue la Fase B se marcará `NOT_RENDERED_HERE` salvo que se produzca evidencia.
2. Defecto previo de `INDEX.md` (arriba) hará fallar cualquier CI Rust que exija verde.
3. PRs #28–#30 cambian `archie-core`, `archie-lan`, `archie-workstation`, `archie-state` y `archie-cli`; pueden mover contratos que esta auditoría cita.

## Fase C, primera rebanada: `archie-shell-bridge` y su consumo en QML

**Rust** (`cargo test -p archie-shell-bridge --locked`): **15 pasan, 0 fallan** (6 unitarias de `client.rs`, 9 de contrato en `tests/contract.rs`). `cargo fmt --check` limpio; `clippy` sin hallazgos en este crate. (`clippy -D warnings` de todo el workspace falla en `archie-orb` por código previo y un clippy 1.98 más estricto que el 1.88 de CI; no es de este PR.)

Casos positivos: contra el **router real de `archie-displayd`** servido por un socket loopback real, con `archied` ausente (`daemon_unreachable`; telemetría presente e independiente de `archied`); grabación nunca presentada como `live`, aunque diga que el daemon era alcanzable; estado `live` con campos de `StatusView` (servido por un par que usa los tipos reales, porque ningún `archied` real puede correr aquí).
Casos negativos: nadie escucha (`displayd_offline`), respuesta que no cumple el esquema, HTTP 500, respuesta de más de 4 MiB, dirección no loopback (rechazada antes de conectar), ruta que podría partir la línea de petición, cuerpo truncado, texto que no es HTTP.

**Binario real** (`archie-shell-bridge watch --once`) contra el `archie-displayd` real en este equipo: trama `daemon_unreachable` con `kernel` presente y `pressure_availability`/`thermal_availability` en `unavailable` con la razón real del kernel; y contra un puerto sin servidor: `displayd_offline`. Ambas se guardaron en `display/shell/prototype/frames/`.

**QML** (`BridgeModelTest.qml`, Qt 6.12, offscreen): **15 comprobaciones, 0 fallos** — estado inicial `waiting`, tramas válidas, grabación ≠ live, razón y edad del dato, rechazo de JSON inválido / otro esquema / trama sin enlace sin pisar la última trama buena, y silencio del bridge tras 3 sondeos. Render del prototipo: 0 avisos de QML.

**Regresión del workspace** (`cargo test --workspace --exclude archie-arena --locked --no-fail-fast`): **1 915 pasan** (1 900 de la línea base + 15 nuevas), **1 falla** (el mismo `index::tests::the_committed_index_is_current` previo) y 78 ignoradas. Sin regresiones.

**No ejecutado / no verificado en esta rebanada:**
- `archied` real: no arranca aquí (`state store unavailable`: falta el almacén PostgreSQL), así que **no existe una trama `live` real**; la trama `live` de las capturas es sintética y está nombrada así.
- Quickshell (`BridgeProcess.qml`), Linux, Wayland, NVIDIA.
- Lectores de pantalla, movimiento reducido en estos componentes, rendimiento.
- Por eso ningún ítem pasa a `TESTED`: `GUI-DISPLAY-001` y `GUI-DISPLAY-002` están `INTEGRATED`; `GUI-CORE-001` y `GUI-CORE-013` `IN_PROGRESS`.

## Fase C, segunda rebanada: inventario LAN y trabajos largos (sólo lectura)

**Rust** (`cargo test -p archie-shell-bridge --locked`): **38 pasan, 0 fallan** en Windows — 6 de `client.rs`, 9 de `tests/contract.rs`, 11 de `tests/lan.rs`, 12 de `tests/jobs.rs`. Una prueba más de `jobs.rs` (`a_symlinked_job_folder_is_never_followed`) es sólo Unix y **no se ejecutó aquí**. `fmt` limpio; clippy: 0 hallazgos en este crate.

- **LAN:** el archivo de entrada se escribe con el **tipo real** `archie_lan::inventory::Inventory`. Casos: lectura correcta, vacío ≠ ausente, desactualizado (límite exacto incluido), hora futura → antigüedad 0, no configurado / ausente / no legible como estados distintos, directorio, JSON roto, otro formato, sin campo de formato, archivo de más de 1 MiB, más de 512 dispositivos (se cuentan, no se envían todos), y la trama lleva `lan` aunque `displayd` esté caído.
- **Trabajos:** `status.json` escrito con el **tipo real** `archie_bridges::jobs::Status`. Casos: el campo libre `latest` **nunca** llega a la shell, sólo se envía el nombre de la carpeta (nunca la ruta), orden por más reciente con los cinco estados, espacio sin trabajos, reloj hacia atrás → inactividad 0, estado dañado o de más de 64 KiB contado como omitido, error truncado a 300 caracteres, tope de 64 trabajos, raíz de espacios listada en cada sondeo, raíz vacía = respuesta vacía, raíz inexistente = se nombra como no legible.

**QML** (`BridgeModelTest.qml`): **21 comprobaciones, 0 fallos** (añade `lan` y `jobs`, y el conteo de trabajos en curso). Render de `SentryPanel` y `ActivityCenter`: 0 avisos de QML. Defectos encontrados en las capturas y corregidos: tarjeta de dispositivo con altura fija que recortaba su tercera línea, y panel de actividad que desbordaba (ahora la altura sale del contenido).

**Binario real:** `archie-shell-bridge watch --lan-file /var/lib/archie/sentry/lan.json --once` en este equipo → `lan: missing` (trama real guardada). Las tramas `synthetic-lan-*` y `synthetic-jobs` son salida **real del binario** sobre archivos de entrada **sintéticos**.

**Regresión del workspace:** **1 938 pasan** (1 900 + 38), **1 falla** (la misma previa de `INDEX.md`), 78 ignoradas. Sin regresiones.

**No ejecutado:** `archie-lan` contra una red real (generaría tráfico SSDP/mDNS en la red del dueño y sus dispositivos acabarían en los fixtures: no se hizo), un `archie-bridges run-job` real, Quickshell, Linux. Estados: `GUI-SENTRY-001` `INTEGRATED`; `GUI-BRIDGE-002` `IN_PROGRESS` (no hay cancelación).

## Rediseño visual Archie Mac (Claude Design)

Sin cambios en Rust ni en el modelo de datos. `BridgeModelTest.qml`: 21 comprobaciones, 0 fallos. Las 10 pantallas del prototipo y dos a 1440p cargan con 0 avisos de QML (Qt 6.12, render por software). Verificación visual: **contra el código de los mockups, no contra una imagen del diseño** (el paquete declara que no se renderizó nunca y sus `.dc.html` no se pueden abrir sin su runtime). Defectos hallados al renderizar y corregidos: texto que se transparentaba a través de paneles sin desenfoque (ahora `Theme.blur`), barra de progreso de la cola pegada al texto, texto del resumen del sistema que se salía de su caja y falta de aire bajo los chips del centro de control. No verificado: Quickshell, desenfoque real, lectores de pantalla, rendimiento. Ver `DESIGN_ARCHIE_MAC.md` §4–5.

## Segunda entrega de diseño (8 superficies, movimiento)

- `BridgeModelTest.qml`: 21 comprobaciones, 0 fallos (el modelo de datos no se tocó).
- Render por software (Qt 6.12, PySide6, `offscreen`) de las 8 superficies nuevas y de sus seis estados, más Vault, paleta y escritorio: **0 mensajes de QML**. Se compararon el escritorio y Vault en cuadrícula con las capturas anteriores: sólo difiere el chip de enlace, porque las anteriores se hicieron con una trama reproducida.
- **No se probó**: que las animaciones se vean y duren como la tabla de movimiento (sólo se comprobó que cargan sin errores); Quickshell; desenfoque real; lectores de pantalla; rendimiento. 
- `SurfacesTest.qml` (nuevo, sin QtTest): 79 comprobaciones, 0 fallos. Construye cada superficie de ejemplo en sus seis estados con tamaño real; exige que las muestras se rotulen «Diseño», que «Permitir» siga deshabilitado con su motivo, que «vacío» no dibuje filas inventadas, que «offline» nunca diga «en vivo», que el estado interno de IRIS no se dibuje en ningún estado y que los cuatro perfiles de movimiento tengan las duraciones del paquete. No prueba el aspecto ni que las animaciones se vean bien. Ejecutado con PySide6 (software, `offscreen`); con `qml` en Linux no está verificado.

## Ventanas, sistema básico y esfera (14 oct)

- `WmTest.qml` (nuevo, sin QtTest): 44 comprobaciones, 0 fallos. Estado inicial, abrir, subir al frente, minimizar y restaurar, clics del dock (abre, minimiza el del frente, sube el de atrás), teclas (abre/cierra), cerrar olvida posición y zoom, todas a la vez, acciones sobre la ventana del frente sin ventanas, la lista del selector, y el arrastre sin salir de pantalla; además las luces y el punto del dock siguen el estado.
- `SurfacesTest.qml`: 79/79 (el selector ahora dice «contenido de ejemplo»). `BridgeModelTest.qml`: 21/21.
- Recorrido con eventos **reales** de ratón y teclado sobre `Main.qml` (arnés fuera del repo): abrir con `H`, arrastrar Vault, minimizar con la luz amarilla, restaurar con el dock, ampliar con la luz verde, cerrar Stage con la luz roja, `Ctrl+M`, `D`, `Ctrl+Mayús+W`, menú del dock, selector con `Tab`+`↵`. Los resultados coinciden con el estado esperado.
- Dos fallos encontrados por ese recorrido y corregidos: (1) la ventana nunca aparecía (su visibilidad dependía de la de su hijo), (2) un `DragHandler` sobre la barra de título se tragaba los clics de las luces.
- Barrido de todas las superficies (14) sin mensajes de QML.
- **No se probó**: rendimiento con varias ventanas abiertas; Quickshell; otro equipo; Linux. La captura repetida de la ventana con la esfera (`grabWindow`) pierde el contexto WebGL: las imágenes con esfera se toman una por arranque.
