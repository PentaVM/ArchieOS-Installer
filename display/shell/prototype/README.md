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
