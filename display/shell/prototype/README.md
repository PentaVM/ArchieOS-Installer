# Prototipo de la shell profesional (Fase B, sólo diseño)

Qt Quick puro, **sin Quickshell**, datos sintéticos rotulados ("DATOS DE DISEÑO"). No habla con
`archied`, `archie-displayd` ni el sistema de archivos. Sustituye nada de `display/native`
(Orbe, Observatorio) ni de `display/stage`.

Ejecutar (Qt 6.6+, Linux o Windows):

```sh
qml display/shell/prototype/Main.qml
```

Teclas: `Ctrl+K` paleta · `C` centro de control · `A` actividad · `G` lista/cuadrícula ·
`M` movimiento reducido · `1..4` tema (frost / workspace / signature / synthesis) · `Esc` cerrar.

Las capturas de `docs/gui/screenshots/` se generaron con Qt 6.12 (PySide6, API gráfica de
software, plataforma `offscreen`, fuente Segoe UI de Windows), a 1920×1080 y 2560×1440.
El script de captura no se versiona (ArchieOS no incluye Python). Los mismos resultados con
`qml` en Linux no están verificados.

**No probado:** Quickshell (`PanelWindow`, capas), blur del compositor, NVIDIA, Wayland,
lectores de pantalla, `qmllint`.
