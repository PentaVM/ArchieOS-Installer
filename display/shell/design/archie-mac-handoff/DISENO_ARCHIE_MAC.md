# Archie Mac · cristal grafito — especificación para implementar

Rediseño **solo visual** de la shell de escritorio de ArchieOS. Es un mac sobrio y silencioso en grafito neutro, flotando sobre un cerebro que piensa (la esfera). Sin neón, números tabulares, y siempre claro de dónde viene cada dato, cuándo, y qué pasó.

> **Regla madre:** no se mueve, quita ni añade ningún elemento de la shell actual. Este paquete cambia colores, materiales, tipografía, iconos, barras, sombras, estados y animaciones. Si algo de aquí parece pedir otro elemento, otra posición o otro comportamiento, **pregunta antes de hacerlo**.

## 1. Qué contiene este paquete

| Archivo | Para qué sirve |
|---|---|
| `tokens.css` | Variables CSS: colores, superficies (cristal y opaco), radios, sombras. Punto de partida de la implementación. |
| `tokens.json` | Los mismos tokens en JSON, por si el programa los consume de otra forma. |
| `componentes.css` | Reglas CSS de todos los componentes, extraídas de los mockups. Requiere `tokens.css`. |
| `icons.json` / `icons.svg` | Los 69 iconos de línea propios (viewBox 24×24). El SVG es un sprite con `<symbol id="ai-…">`. |
| `referencia-pantallas/` | Los mockups fuente (`.dc.html`) de las 10 pantallas y `canvas.json`. **Son referencia visual y de estructura**, no código para copiar: usan un runtime de plantillas propio (`{{…}}`, `<sc-for>`). Léelos para ver la estructura exacta, tamaños y textos. |

Las pantallas aún **no se han abierto ni renderizado**: todo lo de este documento sale del código de los mockups, no de una captura. Antes de dar nada por bueno, compara contra el canvas publicado.

## 2. Principios que no se negocian

1. **Neutral y silencioso.** El color es información, no decoración. Los chips son neutros por defecto; solo **advertencia**, **error** y **Archie** llevan tinte de fondo.
2. **Estado = icono + palabra.** Nunca solo color. Cada estado tiene su forma de icono y su texto.
3. **Lo desconocido se ve distinto.** Cualquier valor sin lectura usa **borde discontinuo** y gris `--unk`.
4. **«No medido» no es cero.** Un medidor sin lectura no es una barra vacía: es una línea discontinua (`.trk.nom`) más la razón escrita debajo.
5. **Cada sección dice su origen**: chip `Real · bridge` (dato real) o `Diseño · …` (dato de ejemplo, borde discontinuo ámbar). Mientras haya datos de ejemplo, aviso fijo «Datos de diseño» bajo la barra.
6. **Lo deshabilitado explica por qué**, en ámbar (`--warn`), junto al control. Nada se aprueba por omisión: en una autorización el foco inicial está en **Rechazar**.
7. **Hora y procedencia visibles**: «Archie lo tocó 14:18», «actualizado hace 2 min», «visto hace 2 min», «sin novedades hace 7 min».
8. **Números tabulares** en todo (`font-variant-numeric: tabular-nums`) para que las cifras no bailen.
9. **Marca de dominio**: círculo `--info` = personal, rombo `--warn` = trabajo. Además lleva texto.

## 3. Tokens

Fuente de verdad: `tokens.css`. Resumen:

**Texto:** `--t1 #f2f1ee` · `--t2 #b9b8c4` · `--t3 #8d8c9c` · `--tdis #5e5d6b`.
**Acentos:** `--acc #6fc8f5` (foco, selección, progreso) · `--arch #f0bb78` (lo hizo Archie).
**Estados:** `--ok #47d18b` · `--warn #f0b429` · `--bad #f2626b` · `--info #6fe3f5` · `--unk #8d8c9c`.

**Superficies de cristal** (todas con borde `--hair` y brillo interior `--inner`):

| Superficie | Cristal | Blur | Opaca |
|---|---|---|---|
| Barra superior | `rgba(16,17,23,.62)` | 30px + saturate 1.4 | `#101117` |
| Dock | `rgba(20,21,28,.55)` | 36px | `#14151c` |
| Ventana | `rgba(18,19,26,.78)` | 40px | `#12131a` |
| Barra lateral | `rgba(26,27,36,.66)` | — | `#1a1b24` |
| Panel (Centro, Actividad, Sentry, paleta) | `rgba(22,23,31,.86)` | 44px | `#16171f` |
| Relieve normal / hover / pulsado | blanco `.05` / `.08` / `.11` | — | `#22232a` / `#2a2b32` / `#31323a` |
| Línea fina / brillo interior | blanco `.09` / `.06` | — | `#2c2d36` / `#25262e` |

**Versión opaca:** misma estructura, `data-surface="opaque"` en la raíz. Sirve para comprobar legibilidad y como modo «reducir transparencia». Todo el texto debe seguir siendo legible sin blur.

**Radios:** ventana 14 · panel 16 · tarjeta 10 · fila 8 · chip 999 · dock 22 · icono del dock 14.

**Categorías de archivo** (aptas para daltonismo): documentos `#e69f00` · medios `#009e73` · datos `#f0e442` · archivo comprimido `#cc79a7` · ejecutable `#d55e00` · otro `#bbbbbb`.

**Fondo:** gradiente neutro provisional + red de nodos (SVG). El fondo real y el color de la esfera los adaptará el equipo; no dependas de ellos.

## 4. Tipografía

- Texto: **Inter** 400/500/600, 13px / 18px de base.
- Títulos y sobrelíneas: **Space Grotesk** 500/600.
- Título de panel: 22/28, 600, `letter-spacing -.015em`. Título de ventana: 17/22, 600, `-.01em`.
- Sobrelínea (`.ovl`): Space Grotesk 600, 10px, `0.12em`, mayúsculas, `--t3`.
- Chip: 11.5px/16 (pequeño 11px). Metadatos: 11px `--t3`. Chip de origen: Space Grotesk 10.5px, `0.08em`, mayúsculas.
- Cifras siempre tabulares.

## 5. Iconos

Juego propio de línea, rejilla **24×24**, `fill:none`, trazo `currentColor`, extremos y uniones redondos. Un solo color por token (el icono hereda `color`).

- El grosor real se mantiene ~1.5px al escalar: se controla con `--sw` según tamaño → 12px:3 · 14px:2.6 · 16px:2.25 · 18px:2 · 20px:1.8 · 28px:1.4 · 36px:1.4 (clases `.i12 … .i36` en `componentes.css`).
- Los **puntos** son segmentos de longitud cero con extremo redondo (`M12 12h.01`) y se pintan con trazo `--sw × 2.2` (clase `.dot`). En `icons.json` van en `dots`; en el sprite, en un segundo `<path class="dot">` dentro del mismo `<symbol>`.
- Uso: `<svg class="ico i16"><use href="#ai-search"/></svg>` (con el sprite incluido en la página).
- Familias: marca (`archie`), barra (`search`, `activity`, `control`…), cromo de ventana (`wClose`, `wMin`, `wZoom`, `close`, `plus`, `chev*`, `more`, `list`, `grid`…), estados (`ok`, `done`, `bad`, `warn`, `waiting`, `unknown`, `stale`, `cancel`, `pause`, `lock`, `sample`…), Vault (`folder`, `doc`, `cad`, `video`, `sheet`, `image`, `archive`, `exe`, `data`, `other`, `recents`, `deliveries`, `workspaces`, `documents`, `trash`, `sealed`), sistema y Sentry (`cpu`, `mem`, `io`, `temp`, `wifi`, `bt`, `moon`, `eye`, `mic`, `camera`, `tv`, `printer`, `speaker`, `devunk`…), dock (`dkOrb`, `dkVault`, `dkForge`, `dkSentry`, `dkStage`, `dkSet`).
- Iconos del dock: blanco sobre cuadrado de 56px, radio 14, fondo de color sólido bajo degradado a 165° (`rgba(255,255,255,.10)` → `rgba(0,0,0,.14)`), sombra interior blanca `.14`. Colores: Orb `#1b8fa6`, Vault `#3b7fd6`, Forge `#b87a0b`, Sentry `#1f9a62`, Stage `#8a5fd6`, Ajustes `#6b7686`.
  - **App abierta:** punto blanco de 4px debajo. **App activa:** contorno `--acc` de 1.5px a 3px de distancia.

## 6. Componentes

Todas las clases están en `componentes.css`. Hoja de referencia visual: `referencia-pantallas/Componentes.dc.html`.

### Chips (`.chip`, 22px; `.chip.sm`, 20px)
Píldora con icono 12px + texto. Neutro: fondo `--raised`, borde `--hair`, texto `--t2`.

- **Enlace con el sistema (7 estados):** `conectando…` (info) · `archied en vivo` (ok) · `GRABACIÓN · no es en vivo` (warn) · `archied no responde · último estado hace N min` (bad) · `displayd sin respuesta` (bad) · `respuesta no válida` (warn) · `bridge sin datos` (warn).
- **Trabajos largos (5):** `iniciando` y `en curso` (info) · `listo` (ok) · `cancelado` (neutro) · `falló` (bad).
- **Autorización (5):** `pendiente` (warn) · `concedida` (ok) · `rechazada` (bad) · `expirada` y `cancelada` (neutros).
- **Origen (obligatorio por sección):** `Real · bridge` (`.chip.src`) · `Diseño · sin servicio` / `· ejemplo` / `· sin contrato` (`.chip.src.dsg`, borde discontinuo ámbar).
- **Archie y control:** `Archie lo tocó <hora>` (`.chip.arch`, tinte `--arch`, hora en `.sub`) · `Autoridad: Archie` · `Autoridad: persona` · `En pausa` (warn).
- **Lecturas ausentes:** `sin lectura` / `no medido` / `sin servicio conectado` (`.chip.unk`, discontinuo, transparente) · `desactualizado · hace 18 min` (warn) · `sin novedades hace 7 min` (neutro).
- Solo `warn`, `bad` y `arch` llevan tinte de fondo (12–13 % del color + borde 34–40 %). El resto: icono con color, fondo neutro.

### Botones y controles
- `.btn` 28px, radio 8, fondo `--hover`. Estados: normal · `.hv` (`--pressed`) · `.pr` (blanco .15 + sombra interior) · `.fc` foco (contorno `--acc` 2px, offset 2) · `.dis` (transparente, borde discontinuo, `--tdis`) · `.ghost` (sin fondo).
- Botón deshabilitado: siempre con `.why` al lado o debajo (11px, `--warn`) explicando la razón.
- Botón de icono de barra `.ibtn` 28×26, radio 7; con contador `.ibtn.cnt`. Pulsado: fondo `--pressed`.
- Conmutador lista/cuadrícula `.seg` + `.segb.on`. Pestañas `.tabs/.tab.on`. Campo de búsqueda `.fld` (40px, radio 10, cursor `--acc` de 1.5px).
- **Semáforo de ventana** `.tl` 12px (grande 26px): cerrar `#d9605a`, minimizar `#d9a441`, zoom `#4fb67b`; ventana inactiva: gris `rgba(255,255,255,.17)`; deshabilitado: solo contorno. El glifo aparece al pasar el ratón.

### Filas
- Fila de Vault `.vrow` 42px (cuadrícula de columnas nombre · tamaño · fecha · contexto), hover `--hover`, seleccionada `--sel`, foco: contorno `--acc` 2px hacia dentro.
- Barra lateral `.srow` 30px; activa: `--sel` con icono `--acc`. Sobrelíneas «Ubicaciones» y «Sistema».
- Paleta `.pr` / `.prow` 44–52px con glifo de 32px, chip de tipo (`App`, `Archivo`, `Acción`) y atajo `↵` en la seleccionada.

### Medidores (`.trk`, 6px)
Pista blanca `.1`, relleno `--acc`; `.w` ámbar, `.b` rojo, `.k` verde; marca de límite `.lim` de 2×12. **Sin lectura:** `.trk.nom` (línea discontinua de 2px, sin pista) + razón debajo (`.why2`). Filas `.mr` de 30px: icono · etiqueta · barra · valor. Temperaturas por zona; una zona sin escala muestra solo el valor.

### Tarjetas (`.cd`, radio 10)
Trabajo largo (título + chip a la derecha, línea de detalle, barra `.prog` con/sin total, error `.err` en rojo con borde), autorización (botón **Rechazar** con el foco + **Revisar detalles…**), dispositivo de Sentry (`.dv`), línea de auditoría (`.aud`: hora de 54px + texto, separadas por línea fina).

### Ventanas y paneles
- Ventana `.win`: radio 14, `--win` + blur, sombra `--sh-window`, barra de título de 52px (semáforo · título Space Grotesk · conmutador).
- Panel `.panel`: radio 16, `--panel` + blur, sombra `--sh-panel`, padding 20 (Sentry 22/24; paleta 0 con cabecera propia).
- Barra superior de 32px; dock flotante centrado con 14px de separación inferior.

## 7. Pantallas (1920×1080) y posiciones

Estas posiciones están **congeladas**; no cambiar el orden ni el contenido.

- **Barra:** izquierda = marca + «Archie» + nombre de app + menús (Archivo, Editar, Vista, Ir, Ventana). Derecha = chip de enlace, «Cámara y micro», «Dominio», búsqueda, contador de actividad, centro de control, reloj.
- **Dock:** Orb, Vault, Forge, Sentry, Stage, Ajustes (en ese orden).
- **Aviso «Datos de diseño»:** arriba a la izquierda, bajo la barra (14, 42).

| # | Pantalla | Archivo | Notas de posición |
|---|---|---|---|
| 1 | Escritorio completo | `Main.dc.html` | Ventana Vault (lista) en 88/100, 1060×720 · ventana Stage en 1210/172, 600×400 |
| 2 | Vault en cuadrícula | `Vault-cuadricula.dc.html` | Vault en 88/92, 1060×840, 4 columnas de fichas, huecos de carga discontinuos |
| 3a | Centro de control, «archied no responde» | `Centro-sin-respuesta.dc.html` | Panel arriba a la derecha, 1456/44, ancho 440. Medidores «no medido» |
| 3b | Centro de control, en vivo | `Centro-en-vivo.dc.html` | Mismo panel con medidores completos |
| 4 | Actividad con trabajos largos | `Actividad.dc.html` | Panel 1436/44, ancho 460. En curso, listo, falló, sin total |
| 5a | Sentry, red local con dispositivos | `Sentry-dispositivos.dc.html` | Panel centrado 570/110, ancho 780, rejilla de 2 columnas |
| 5b | Sentry, escáner sin ejecutar | `Sentry-sin-escaner.dc.html` | Mismo panel; contador «—», ruta `/var/lib/archie/sentry/lan.json` |
| 6 | Paleta de comandos | `Paleta.dc.html` | Centrada, 600/140, ancho 720 |
| 7 | Hoja de componentes | `Componentes.dc.html` | 1920×3400 |
| 8 | Escritorio opaco | `Opaca.dc.html` | Pantalla 1 con `opaque` fijo |

Texto en español, hora de ejemplo «lun 12 oct 14:32». Todo el contenido de ejemplo está marcado como `Diseño · …`; al conectar datos reales hay que cambiar el chip a `Real · bridge`.

## 8. Cómo implementarlo (orden sugerido)

1. Importa `tokens.css` y `componentes.css`; incluye el sprite `icons.svg` en el documento; carga Inter y Space Grotesk.
2. Pon `class="archie-root"` en el contenedor raíz de la shell. Para el modo opaco, `data-surface="opaque"`.
3. Sustituye estilos existentes **sin tocar estructura ni orden de los elementos**: primero tokens y tipografía, luego barra y dock, ventanas y paneles, y por último estados y chips.
4. Cada estado debe mostrar icono **y** texto. Revisa que ninguna información dependa solo del color.
5. Comprueba la versión opaca antes de cerrar.

## 9. Pendiente y puntos sin verificar

- **Nada se ha renderizado todavía.** Riesgos conocidos: altura de la hoja de componentes (3400px), si caben tres filas en la cuadrícula del Vault (840px), si el panel de Actividad cabe en 1080px, y un color interpolado con `{{ }}` en la hoja de componentes.
- **Animaciones:** no hay tabla de movimiento definida aún. Hasta que se defina, nada se anima salvo transiciones simples de estado (hover/pulsado/foco). Respetar `prefers-reduced-motion`.
- **No incluido todavía:** versiones 2560×1440, el resto de superficies (10–17), lista de decisiones, tabla de movimiento y hoja de tokens/tipografía/radios/elevación.
- **Fondo y esfera:** pendientes de adaptar por el equipo; los valores del fondo aquí son provisionales.
- Los nombres de clase (`.chip`, `.cd`, `.dv`, `.trk`…) son los de los mockups y en algunos casos son locales de cada pantalla; renómbralos a tu convención si hace falta.
