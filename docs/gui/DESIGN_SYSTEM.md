# Sistema de diseño de la GUI profesional

Estado: **Fase B (diseño).** Implementado como tokens en `display/shell/prototype/Theme.qml` y componentes de ejemplo. Renderizado verificado con Qt 6.12 (PySide6, API gráfica de software, plataforma `offscreen`). **No verificado:** Quickshell, blur real del compositor, GPU NVIDIA, fuentes del sistema final.

## 1. Tres direcciones y la síntesis

Las tres direcciones del handoff se exploraron como **juegos de tokens** (color, radio, densidad) sobre la misma estructura de pantallas. Diferencian la sensación, no el layout; una dirección con layout propio exigiría otro prototipo.

| Dirección | Idea | Radio | Densidad | Acento | Cuándo gana |
|---|---|---|---|---|---|
| **Professional Frost** | sobria, grafito azulado, mucho aire (macOS/COSMIC) | 14 | 1,00 | `#5aa9ff` | uso diario, pantallas grandes |
| **Adaptive Workspace** | compacta y modular (KDE + Noctalia) | 8 | 0,86 | `#3ec9a7` | trabajo intensivo, resoluciones modestas |
| **Archie Signature** | negro profundo de la identidad del Observatorio, cian del Orbe | 12 | 1,00 | `#6fe3f5` | coexistencia visual con el Orbe |
| **Síntesis (propuesta final)** | fondo/superficies de Frost, **acento azul-cian a medio camino** (`#6fc8f5`) para vincularse con el Orbe sin competir con él, radio 12, densidad 0,94 | 12 | 0,94 | `#6fc8f5` | por defecto |

Razón de la síntesis: las superficies de Signature (`#04060a`) son casi negro puro y cansan en uso prolongado con ventanas de texto; Frost añade jerarquía de superficies; el acento cercano al cian del Orbe (`#6fe3f5`, `display/native/qml/Main.qml`) hace que cambiar entre Orbe y Workspace se lea como un mismo sistema. La densidad 0,94 es un punto medio y es un token: el usuario puede elegir 0,86 / 1,00. Es una **propuesta**; la elección final es de Xavi.

## 2. Tokens

### Color (síntesis)

| Token | Valor | Uso |
|---|---|---|
| `bg` | `#080b10` | fondo de contenido |
| `surface` | `#11161d` (88 % con vidrio, 100 % sólido) | ventanas, paneles |
| `raised` | `#192029` | tarjetas, cabeceras |
| `line` | `#252d38` | bordes de 1 px |
| `ink` / `soft` / `dim` | `#eaeff5` / `#a8b1be` / `#8792a2` | texto primario / secundario / terciario |
| `accent` | `#6fc8f5` | foco, selección, progreso |
| `ok` `warn` `bad` `info` `unknown` | `#47d18b` `#f0b429` `#f2626b` `#6fe3f5` `#8792a2` | estados |

**Contraste calculado (WCAG, sobre `surface`)**, síntesis: `ink` 15,7 · `soft` 8,4 · `dim` 5,8 (5,2 sobre `raised`) · `accent` 9,7. Estados sobre `#11161d`: ok 9,3 · warn 9,7 · bad 5,8 · info 12,1. Las cuatro variantes cumplen ≥ 4,5:1 en texto (AA). No se midió contraste de texto sobre el fondo de pared ni sobre contenido translúcido real.

**Regla heredada de `display/visual/palette.hpp`:** el color nunca es el único portador de significado. Cada estado lleva glifo y palabra (`✓ listo`, `! necesita aprobación`, `○ sin conexión`). La frescura no toca el matiz: un dato viejo no se atenúa hacia "calma".

### Tipografía

Sin fuente propia todavía. El prototipo usa la del sistema (en la captura: Segoe UI porque se renderizó en Windows). Decisión pendiente: familia sans de sistema + cifras tabulares (`font.features: {"tnum": 1}`) para tamaños, horas y progreso. Tamaños base (× densidad): pequeño 11, cuerpo 13, título 16. Truncado: `elide` a derecha para nombres, al centro para rutas largas.

### Geometría

Radios 8 / 12 / 14 según dirección; huecos 10 px × densidad; barra superior 32; dock 60; objetivos táctiles ≥ 32 px en controles interactivos del dock y de las filas (objetivo táctil opcional; la barra superior compacta es sólo ratón/teclado y queda por mejorar).

### Elevación y vidrio

- Sombra: dos rectángulos apilados, sin shader (coste cero de GPU mientras corre IRIS).
- **Vidrio**: `Theme.glass` aplica 88 % de opacidad; el blur real depende del compositor y **no está implementado**. El fallback accesible es el 100 % sólido. Los paneles superpuestos (centro de control, actividad, paleta) usan siempre sólido: con el 88 % se veía el contenido de detrás a través del texto (defecto detectado en las capturas y corregido).
- Sin bloom ni resplandor global en la interfaz de productividad.

### Estados de componente

hover · pressed · selected · disabled · focused · pending · success · warning · danger. En el prototipo: seleccionado = relleno 22 % de acento + borde de acento; deshabilitado = texto `dim` + motivo real en `warn` (ver "Pausa de emergencia"); foco = borde de acento de 2 px.

### Movimiento

Ver `MOTION_CINEMATICS.md`. Tokens: `fast` 120 ms, `base` 180 ms, `slow` 280 ms; todos pasan a 0 con `reducedMotion`.

### Responsive

Layout proporcional al tamaño de la ventana (verificado en 1920×1080 y 2560×1440). **No verificado:** HiDPI con escala fraccional, varios monitores, 1366×768.

### Internacionalización

El prototipo está en español y **con cadenas escritas en QML**. Para el producto: ES/EN con `qsTr()` y archivos `.ts`; pendiente (ítem de implementación, no hecho).

## 3. Componentes del prototipo

`Panel` (superficie), `Chip` (estado con glifo+palabra), `DesignBanner` (aviso de datos sintéticos), `TopBar`, `Dock`, `VaultWindow`, `ControlCenter`, `ActivityCenter`, `CommandPalette`. Todos en Qt Quick puro (sin Quickshell) para poder probarlos sin él.

## 4. Principios de honestidad en la interfaz (verificables en las capturas)

1. Todo dato del prototipo se rotula **"DATOS DE DISEÑO · no son del sistema"** y la barra superior dice **"sin conexión a archied"**.
2. Lo no medido se dibuja como regla discontinua con "no medido: sin lectura", no como barra a cero (RFC-006).
3. La **pausa de emergencia** está deshabilitada con el motivo real ("archied no expone este control todavía"): no se dibuja un botón que no hace nada.
4. **Sealed** aparece como fila de estado, nunca como carpeta navegable.
5. En autorizaciones, **Rechazar** es el botón de foco por defecto y no existe aprobación implícita.
6. El indicador "Cámara y micro inactivos" está pensado para derivarse de `CameraAuthority` (`GUI-SENTRY-002`), no del estado de la UI.
