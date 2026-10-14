# Plan de implementación por etapas

Spec: `docs/superpowers/specs/2026-10-14-native-shell-integration.md`.
PR41 inicial: eeeb68c. Ejecución directa con auditorías independientes por dominio;
no pausas de permiso: el propietario ya autorizó implementación y push, no merge.

## Restricciones globales

Mantener Core/Policy/leases/voz; APIs read-only de displayd; Qt mínimo 6.6; sin
WebEngine/Python en arranque operativo; ningún merge/force; tema claro al final.
Fuentes ausentes no son exitosas ni vacías. Fixtures nunca se marcan live.

## Foco de revisión

- Payload parcial/estado desconocido: rechazo atómico y frescura conservada.
- Pérdida de bridge: mostrar silencio sin invalidar indebidamente LAN/jobs propios.
- Vista oculta/cambio de lens: timers detenidos y respuesta antigua descartada.
- Enter/ratón/dominio: sólo acciones visibles/elegibles; no mutación por muestra.
- Sin motor/module externo: arranque Qt Quick con fallback honesto y sin Chromium.

## 1. Contratos reales

Files: `archie-displayd/src/{bridge,snapshot,lib}.rs` (o DTO dedicado),
`archie-shell-bridge/src/protocol.rs`, sus tests de rutas/contract/reconnect.
Transportar providers, Cortex DTO y workflows del Orb con estado y source explícitos;
mantener Frame schema 1, compatibilidad de campos ausentes y límites.

- [ ] Escribir y ejecutar tests que fallen por proyecciones ausentes.
- [ ] Implementar lecturas de contratos existentes, sin requests mutantes.
- [ ] Probar live/fixture/offline/partial y redacción de contexto privado.
- [ ] Ejecutar cargo test de displayd/shell-bridge y fmt/clippy correspondientes.

## 2. Host y viewport nativos

Files: `display/native/{CMakeLists.txt,src/main.cpp,src/ShellRuntime.*,qml/OrbViewport.qml}`,
LumenField/ArchieBridge cuando sea necesario, tests nativos.
`--shell` hospeda ventana Qt Quick sobre recursos locales. Un bridge watch tipado
provee Frame por QProcess acotado; no duplica Core. Viewport reutiliza LumenScene.

- [ ] Tests RED de lifecycle/framing y carga sin QtWebEngine.
- [ ] Implementar viewport/host compartidos y paro/reconexión de proceso de lectura.
- [ ] Aplicar budget/presentación/reduced motion y descartar resultados obsoletos.
- [ ] Preservar control cinematic/overlay/Antes y probar protocolo nativo.
- [ ] Build CMake, CTest, recursos instalados y render-check.

## 3. UI conectada y acciones veraces

Files: `BridgeModel`, `Main`, `LiveOrb`, `InspectorPanel`, `SettingsWindow`,
`ForgeWindow`, `NotificationCenter`, `ActivityCenter`, `VaultWindow`, `SearchPanel`,
`WindowSwitcher`, `WorkspaceMenu`, `Panel` y tests QML.

- [ ] Tests RED de frescura/tipos/acciones ocultas/autoridad/deshacer inexistente.
- [ ] Consumir campos del bridge real en panels; read-only o disabled donde falte canal.
- [ ] Quitar simulación operativa, selección de dominio ficticia y claims de autoridad.
- [ ] Preservar UX visual/menu/teclado; arreglar Escape y QColor.
- [ ] Render normal/degraded/offline; suites QML y contrato real de bridge.

## 4. Presets/voz y despliegue

Files: presets/control/CLI/voice si precisan extensión, servicio/session/build/docs.

- [ ] Presets nuevos aditivos, voces cinematic/overlay conservadas; tests RED/GREEN.
- [ ] Session launch del componente nativo sin privilegios nuevos.
- [ ] Limpiar documentación y separar visor externo experimental del arranque real.
- [ ] Registrar matriz de capacidades conectadas/no expuestas y comandos reproducibles.

## 5. Modo claro beta (último)

Files: `Theme.qml`, Apariencia y bindings semánticos de paneles/viewport, tests QML.

- [ ] Test RED de selector y tokens de contraste/navegación compartida.
- [ ] Añadir paleta clara centralizada y selector beta, preservar oscuro por defecto.
- [ ] Render/verificar textos, selección, diálogos y foco claro/oscuro.

## 6. Cierre sin merge

- [ ] Pruebas pertinentes, revisión independiente de todo el diff y fixes comprobados.
- [ ] Commits por etapas, push fast-forward al PR41 y bitácora/pendientes actualizados.
- [ ] PR sigue Draft/open, main y release intactos. Nunca afirmar GPU/operaciones
  físicas verificadas por fixtures.
