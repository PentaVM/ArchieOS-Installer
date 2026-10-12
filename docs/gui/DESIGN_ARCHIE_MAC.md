# Diseño Archie Mac aplicado a la shell

Estado: **rediseño visual aplicado al prototipo Qt Quick** (`display/shell/prototype`). Sustituye por completo el aspecto anterior (las tres direcciones Frost / Workspace / Signature y su síntesis, descritas en versiones anteriores de este archivo). No cambia ninguna función, ningún comportamiento y ninguna posición **salvo las que se aprobaron expresamente** (ver §3).

## 1. Origen

Entregado por Claude Design a partir del sistema de diseño "Archie Mac" (paquete `archie-mac-handoff`, copiado sin modificar a `display/shell/design/archie-mac-handoff/`):

- `DISENO_ARCHIE_MAC.md`: especificación; `tokens.css` / `tokens.json`; `componentes.css`; `icons.json` / `icons.svg` (69 iconos de línea 24×24); `referencia-pantallas/` (10 mockups `.dc.html`, que usan un motor de plantillas propio y no se pueden abrir sin su `support.js`, que no viene en el paquete).
- **El propio paquete declara que ninguna pantalla se abrió ni se renderizó**: todo sale del código de los mockups. Por eso lo aplicado se comparó contra el *código* de los mockups (estructura, tamaños, textos), **no contra una imagen del diseño**.

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
- Las 8 superficies que el sistema pidió y el paquete no incluye (Alt-Tab, notificaciones, Forge, inspector de IRIS/Cortex, Ajustes, bloqueo, coexistencia con el Orbe, búsqueda), la tabla de movimiento, la lista de decisiones y las versiones 2560×1440 de todas las pantallas: **el propio paquete los declara pendientes**. Se renderizaron a 1440p sólo el escritorio y la actividad.
- La esfera: no se tocó ni se ha visto renderizada; el fondo del prototipo es el provisional del diseño.

## 6. Capturas

`docs/gui/screenshots/`: `01-escritorio`, `02-vault-cuadricula`, `03-centro-sin-respuesta` (trama **real**), `04-centro-en-vivo` (trama **sintética**), `05-actividad` (bridge real sobre un espacio de trabajo sintético), `06-sentry-dispositivos` (inventario sintético), `07-sentry-sin-escaner` (trama **real**), `08-paleta`, `09-opaca`, `10-cristal-con-blur`, y `01-escritorio-1440p`, `05-actividad-1440p`. Generadas con Qt 6.12 (PySide6, render por software, `offscreen`).
