# Diseño Archie Mac aplicado a la shell

Estado: **rediseño visual aplicado al prototipo Qt Quick** (`display/shell/prototype`). Sustituye por completo el aspecto anterior (las tres direcciones Frost / Workspace / Signature y su síntesis, descritas en versiones anteriores de este archivo). No cambia ninguna función, ningún comportamiento y ninguna posición **salvo las que se aprobaron expresamente** (ver §3).

## 1. Origen

Entregado por Claude Design a partir del sistema de diseño "Archie Mac" (paquete `archie-mac-handoff`, copiado sin modificar a `display/shell/design/archie-mac-handoff/`):

- `DISENO_ARCHIE_MAC.md`: especificación; `tokens.css` / `tokens.json`; `componentes.css`; `icons.json` / `icons.svg` (69 iconos de línea 24×24); `referencia-pantallas/` (10 mockups `.dc.html`, que usan un motor de plantillas propio y no se pueden abrir sin su `support.js`, que no viene en el paquete).
- **La primera entrega declaraba que ninguna pantalla se había renderizado**: todo salía del código de los mockups, y se comparó contra el *código* (estructura, tamaños, textos), no contra una imagen. La **segunda entrega** (§8) dice haber renderizado y revisado sus pantallas nuevas en un navegador de pruebas; yo tampoco he visto esos renders (los `.dc.html` no se abren sin `support.js`, que no viene), así que lo aplicado sigue comparándose contra el código.

## 2. Qué se aplicó

| Pieza | Dónde |
|---|---|
| Tokens (cristal y opaco), fuentes, radios, categorías, dock, semáforo | `Theme.qml` |
| 69 iconos de línea (generados desde `icons.json`) y su renderizador (trazo por tamaño, puntos como segmentos de longitud cero) | `Icons.qml` (no editar a mano), `Icon.qml` |
| Fuentes Inter y Space Grotesk (variables, SIL OFL 1.1) | `display/shell/fonts/` |
| Chip (neutro por defecto; sólo aviso, error y Archie con tinte; discontinuo para "sin lectura" y "Diseño"), borde discontinuo | `Chip.qml`, `DashedRect.qml` |
| Superficie de cristal (ventana, panel, dock) | `Panel.qml` |
| Barra superior, dock, aviso "Datos de diseño", fondo provisional | `TopBar.qml`, `Dock.qml`, `DesignBanner.qml`, `Background.qml` |
| Vault (lista y cuadrícula), Stage, Centro de control, Actividad, Sentry, Paleta | `VaultWindow.qml`, `Main.qml`, `ControlCenter.qml`, `ResourceGauges.qml`, `ActivityCenter.qml`, `SentryPanel.qml`, `CommandPalette.qml` |

Los datos reales (adaptador `archie-shell-bridge`) y su modelo (`BridgeModel.qml`) no se tocaron: sólo cambió cómo se dibujan. `BridgeModelTest.qml`: 21 comprobaciones, 0 fallos.

## 3. Cambios de posición, tamaño o contenido (aprobados por el dueño)

Posiciones del diseño en 1920×1080: Vault 88/100, 1060×720 en lista y 88/92, 1060×840 en cuadrícula; Stage a 110 px del borde derecho, 600×400; Centro de control 440 px y Actividad 460 px a 24 px del borde derecho; Sentry centrado, 780 px, dos columnas; Paleta centrada, 720 px.
Añadidos aprobados: pie de Vault; segunda pestaña de ejemplo con ×, y flecha de orden en «Nombre» (indican comportamientos que aún no existen); textos explicativos de Sentry, de «Resumen del sistema oculto» y el recuadro del compositor de Stage; veredicto de presión, ventanas de 60 s y 300 s y marcas de límite térmico (datos reales que antes no se mostraban).

## 4. Correcciones a la verdad (aprobadas) y desviaciones del diseño

Donde el diseño muestra algo que el sistema no sabe, se muestra lo que sí se sabe:

- **«visto hace 2 min» por dispositivo** (Sentry): el sistema sólo sabe la hora del escaneo, no la de cada dispositivo. Se muestra una vez, en la cabecera: «inventario actualizado hace N».
- **«presupuesto sin lectura»** con el daemon caído: la telemetría llega por otro camino y el presupuesto es real; se muestra el real («balanced») y sólo dice «sin lectura» si no llegó telemetría.
- **«último estado hace N»** en el chip de enlace: sólo si existe un estado previo (no se dice que hubo un último estado cuando nunca lo hubo).
- Si no hay espacios de trabajo configurados, el contador de actividad muestra «—», no un número.

Desviaciones técnicas (Qt Quick no puede reproducirlo igual):

- **Sin desenfoque.** La shell no desenfoca; lo hace el compositor. `Theme.blur` (apagado por defecto; tecla `B` en el prototipo) hace que ventana, barra lateral y paneles usen su relleno **sólido** cuando no hay desenfoque real: con la translucidez del diseño y sin blur, el texto de lo que hay detrás se ve a través del panel (se comprobó en las capturas). Barra y dock siguen siendo de cristal. La captura `10-cristal-con-blur` muestra el aspecto con translucidez (sin blur real).
- **Medios píxeles de tamaño de letra** (12,5 · 11,5 · 10,5 · 13,5 px): `font.pixelSize` de Qt es entero; se redondean hacia arriba.
- **Sombras**: aproximadas con capas translúcidas apiladas; no son idénticas al difuminado del diseño.
- **Iconos del semáforo al pasar el ratón, flecha de orden, × de las pestañas, cambio de pestañas y arrastre**: dibujados pero sin comportamiento (Vault no tiene backend).
- **Animación**: el diseño no define tabla de movimiento («hasta que se defina, nada se anima salvo hover/pulsado/foco»). Sólo hay el escalado del dock y el deslizamiento del conmutador lista/cuadrícula; todo a 0 con «movimiento reducido».

## 5. No verificado

- Quickshell, Wayland, NVIDIA; desenfoque real del compositor; lectores de pantalla; contraste medido sobre el fondo real (el diseño lo asume sobre `--desktop`); rendimiento.
- Las 8 superficies, la tabla de movimiento y la lista de decisiones llegaron en la segunda entrega (§8). Se renderizaron a 1440p sólo el escritorio y la actividad; las versiones 2560×1440 del paquete no se han aplicado ni comparado (son la misma shell a escala 1,333×).
- La esfera: no se tocó ni se ha visto renderizada; el fondo del prototipo es el provisional del diseño.

## 6. Capturas

`docs/gui/screenshots/`: `01-escritorio`, `02-vault-cuadricula`, `03-centro-sin-respuesta` (trama **real**), `04-centro-en-vivo` (trama **sintética**), `05-actividad` (bridge real sobre un espacio de trabajo sintético), `06-sentry-dispositivos` (inventario sintético), `07-sentry-sin-escaner` (trama **real**), `08-paleta`, `09-opaca`, `10-cristal-con-blur`, y `01-escritorio-1440p`, `05-actividad-1440p`. Generadas con Qt 6.12 (PySide6, render por software, `offscreen`).

## 8. Segunda entrega: 8 superficies, movimiento y decisiones

Paquete `archie-mac-handoff actualizado` (copiado sin modificar a `display/shell/design/archie-mac-handoff/`). Tokens idénticos; cambian `componentes.css` (selectores de botón con `:where`), los iconos (81, +12: `bell`, `windows`, `plug`, `trace`, `key`, `cloud`, `power`, `lines`, `stack`, `spaces`, `bug`, `enter`; los 69 anteriores no cambian), y se añaden `MOVIMIENTO.md`, `DECISIONES.md`, `assets/esfera-crop.jpg` y los mockups nuevos (8 superficies × normal / 2560 / estados, más movimiento y decisiones).

**Aplicado** (todo sobre `display/shell/prototype`; teclas en el `README.md` de esa carpeta):

| Superficie | Archivo | Tecla |
|---|---|---|
| Alt-Tab / vista general | `WindowSwitcher.qml` | `W` |
| Notificaciones: centro, avisos emergentes, diálogo de autorización | `NotificationCenter.qml`, `Toasts.qml`, `AuthDialog.qml` | `N`, `T` |
| Forge | `ForgeWindow.qml` (también desde el dock) | `F` |
| Inspector IRIS / Cortex | `InspectorPanel.qml` | `I` |
| Ajustes (4 secciones) | `SettingsWindow.qml` (también desde el dock) | `,` |
| Bloqueo, arranque, asistente de instalación | `LockScreen.qml` | `L`, `Mayús+L` |
| Selector de espacio de trabajo + hueco del Orbe | `WorkspaceMenu.qml`, `OrbHole.qml` | `X` o el chip «Dominio» |
| Búsqueda | `SearchPanel.qml` | `/` |

Cada una dibuja sus estados (normal, vacío, error, offline, degradado, desactualizado) y el anillo de foco visible, gobernados por `Demo.qml` (`V` cambia de estado, `.` el foco, `H` deja el escritorio vacío como en los tableros del diseño). Componentes compartidos: `StateChip`, `VoidBox`, `ErrBox`, `NoteBox`, `Why`, `Btn`, `FootHints`, `Kbd`, `Eyebrow`, `ListRow`, `AppWindow`, `SideRow`, `DomainDot`. Capturas: `screenshots/11` a `20` (normal) y `21` a `28` (hojas de seis estados).

**Movimiento** (`MOVIMIENTO.md` → `Theme.qml`): cuatro perfiles (Full, Balanced, Quiet, Movimiento reducido) con las duraciones y curvas del paquete; sólo `transform` y `opacity`. Paneles y ventanas aparecen y desaparecen con fundido más deslizamiento o escala (`Panel.shown`); el dock y el conmutador lista/cuadrícula usan los mismos perfiles; en Quiet no hay escalado y en reducido todo es instantáneo. `P` cambia de perfil, `M` fuerza movimiento reducido. **No hay disparo real** para copia con progreso, diálogo de autorización ni etapas de Forge: se dibujan rotulados «Diseño».

**Correcciones a la verdad** (misma regla aprobada para la primera entrega): el diseño rotula casi todo «Real · bridge» o «Real · shell», pero el bridge no entrega ventanas, historial de notificaciones, catálogo de puentes, trayectorias de Cortex, proveedores, modelos, política de apps ni diagnóstico. Todas esas superficies dicen **«Diseño · datos de ejemplo»**. La búsqueda no lleva chip propio, igual que el diseño: la rotula el aviso «Datos de diseño» de la shell. Los trabajos largos reales del bridge siguen viéndose en el panel de Actividad, no en estas muestras.

**Cambios de comportamiento que el diseño introduce, aprobados por el dueño** (no son sólo visuales):
- El chip «Dominio» de la barra ahora abre el selector de espacio (decisión 5 del paquete). El chip sigue diciendo «sin lectura»; el menú es de ejemplo.
- Los iconos Forge y Ajustes del dock abren sus ventanas de ejemplo.
- La búsqueda (`/`) es una superficie distinta de la paleta (`Ctrl+K`): la lupa de la barra sigue abriendo la paleta (el paquete no aclaraba cuál de las dos debía abrirla; se mantiene lo que ya hacía).
- El perfil de movimiento por omisión es **Balanced** (recomendación del paquete); antes la animación era única.

**Desviaciones:** la tipografía monoespaciada es la del sistema (el diseño usa `ui-monospace`); la ventana de Ajustes tiene alto fijo de 520 (el diseño dice `min-height`); `OrbHole` sólo se ve con el escritorio vacío (`H`) para no tapar Vault; sombras aproximadas como antes; sin versiones 2560×1440 propias.

### Decisiones abiertas del dueño (`DECISIONES.md`)

Ninguna cambia lo entregado hasta que se elija. Lo que hizo el prototipo mientras tanto:

| # | Decisión | Recomendación del diseño | Estado en el prototipo |
|---|---|---|---|
| 1 | Escala en 2560×1440 | seguir la escala del sistema | sin resolver; se renderiza proporcional |
| 2 | Fondo con la esfera | campo 3D en vivo, imagen de respaldo | **no tocado**: fondo provisional de nodos; `assets/esfera-crop.jpg` queda en el paquete y no se usa |
| 3 | Color de la esfera frente al grafito | dejarlos | sin efecto aquí |
| 4 | Alt-Tab con miniaturas | icono + título | aplicado icono + título |
| 5 | Dónde vive el selector de espacio | sólo en el chip «Dominio» | aplicado (ver arriba) |
| 6 | Escalada de autorización | diálogo central | aplicado como diálogo de ejemplo con «Permitir» deshabilitado |
| 7 | Agrupar notificaciones | por origen | aplicado por origen |
| 8 | Indicador del arranque | lista de servicios | aplicado, sin porcentaje |
| 9 | Modo opaco | automático + ajuste manual | sólo el ajuste manual (`O`); no lee la preferencia del sistema |
| 10 | Perfil de movimiento por omisión | Balanced | aplicado Balanced |
| 11 | Alojamiento de tipografías | empaquetarlas | ya empaquetadas (`display/shell/fonts`) |

### No verificado en la segunda entrega

Quickshell, desenfoque real, lectores de pantalla, rendimiento y que el movimiento se vea como el diseño (no hay referencia renderizada). Las animaciones se comprobaron sólo por ausencia de errores de QML; no se midió su duración real.

## 9. Ventanas, sistema básico y color de la esfera (14 oct, pedido del dueño)

El dueño pidió, tras probar el prototipo en Windows: quitar el velo gris y el polvo de la esfera, que se genere bien con muchos
archivos y se parezca a la idea base, mover/cerrar/minimizar ventanas y activar lo que estaba sin usar. Esto **no es sólo visual**:
cambia comportamiento y va aquí para que se vea qué se pidió y qué se hizo.

| Qué | Cómo | Límite honesto |
|---|---|---|
| Gestor de ventanas | `Wm.qml` (estado), `WinHost.qml` (coloca, apila, sube al frente), `Lights.qml` (luces), `TitleDrag.qml` (arrastre), `StageWindow.qml` | El contenido de cada ventana sigue siendo de ejemplo. No hay compositor: es el de esta shell |
| Forge, Ajustes y Sentry pasan de «panel que se cierra al hacer clic fuera» a ventana persistente | Sentry gana una barra de título con luces (antes no tenía) | Cambia su aspecto: 52 px más de alto |
| Luces del semáforo | Roja cierra, amarilla minimiza (antes cerraba), verde amplía | Estaban dibujadas pero sin función, salvo en Vault |
| Dock | Punto por ventana abierta, atenuado si está minimizada; clic abre/restaura/sube/minimiza; clic derecho abre `DockMenu.qml` | |
| Selector de ventanas (`W`) | Lista las ventanas reales y se maneja con `Tab`/`Mayús+Tab`/`↵` | Su contenido y el reparto Trabajo/Personal siguen siendo de ejemplo; el chip lo dice |
| Reloj | Hora y fecha reales en la barra y en el bloqueo (`Demo.qml`) | Antes eran el texto fijo del diseño («lun 12 oct 14:32») |
| Silencio | Único interruptor del Centro de control que funciona: oculta los avisos de esta shell | Wi-Fi, Bluetooth y Privacidad siguen deshabilitados («sin servicio conectado»): no hay servicio detrás |
| Bloqueo | Botón «Volver al escritorio» | **No autentica.** Lo dice la propia pantalla |
| Ajustes > Apariencia | Perfil de movimiento, movimiento reducido, opacidad, cristal | Antes sólo existían como teclas |
| Esfera: velo y polvo | `ambient` («polvo de luz») apagado, exposición 1,0 (de fábrica 1,5) y fondo negro | Se ajusta a través de los controles del propio motor; el motor no se toca. El efecto del polvo se vio al quitarlo; el de la exposición no se midió por separado |
| Esfera: más nodos | 2 500 visibles (de fábrica 220) y 30 archivos por carpeta | Medido: 60 FPS con 2 500 nodos en la GPU de este equipo. Con 5 000 no se midió |
| Esfera: color | «Holograma frío» (filtro CSS de la shell: giro de tono hacia cian, más saturación y contraste); «Vivo» (mismos tonos); «Original» | Con el contenido de este repo el motor dibuja naranja y crema (en su código, las carpetas de primer nivel se tiñen con el acento cálido). Ningún ajuste del motor que se probó lo cambió (acento cálido, paleta por tipo, esquema de color). El filtro gira **toda** la imagen, así que las guías y el cono salen rosados |
| Foco del teclado | Con la página de la esfera cargada, el teclado se lo quedaba ella y las teclas de la shell no respondían; ahora el foco vuelve a la shell | |

Verificado: `WmTest.qml` (44) y recorrido con eventos reales de ratón y teclado (arrastrar, minimizar, restaurar desde el dock, zoom, cerrar,
menú del dock). No verificado: rendimiento con muchas ventanas, 5 000 nodos, otro equipo/GPU, Linux.

Capturas: `29` escritorio con la esfera en «Holograma frío» (Qt 6.12 + QtWebEngine, GPU real; una captura por arranque), `30` cinco ventanas apiladas con la del frente
en color, `31` menú del dock, `32` Ajustes > Visor gráfico.
