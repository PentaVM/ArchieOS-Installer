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

- [x] Escribir y ejecutar tests que fallen por proyecciones ausentes.
- [x] Implementar lecturas de contratos existentes, sin requests mutantes.
- [x] Probar live/fixture/offline/partial y redacción de contexto privado.
- [x] Ejecutar cargo test de displayd/shell-bridge y fmt/clippy correspondientes.

## 2. Host y viewport nativos

Files: `display/native/{CMakeLists.txt,src/main.cpp,src/ShellRuntime.*,qml/OrbViewport.qml}`,
LumenField/ArchieBridge cuando sea necesario, tests nativos.
`--shell` hospeda ventana Qt Quick sobre recursos locales. Un bridge watch tipado
provee Frame por QProcess acotado; no duplica Core. Viewport reutiliza LumenScene.

- [x] Tests RED de lifecycle/framing y carga sin QtWebEngine.
- [x] Implementar viewport/host compartidos y paro/reconexión de proceso de lectura.
- [x] Aplicar budget/presentación/reduced motion y descartar resultados obsoletos.
- [x] Preservar control cinematic/overlay/Antes y probar protocolo nativo.
- [x] Build CMake, CTest, recursos embebidos y render-check (instalación Linux pendiente).

## 3. UI conectada y acciones veraces

Files: `BridgeModel`, `Main`, `LiveOrb`, `InspectorPanel`, `SettingsWindow`,
`ForgeWindow`, `NotificationCenter`, `ActivityCenter`, `VaultWindow`, `SearchPanel`,
`WindowSwitcher`, `WorkspaceMenu`, `Panel` y tests QML.

- [x] Tests RED de frescura/tipos/acciones ocultas/autoridad/deshacer inexistente.
- [x] Consumir campos del bridge real en panels; read-only o disabled donde falte canal.
- [x] Quitar simulación operativa, selección de dominio ficticia y claims de autoridad.
- [x] Preservar UX visual/menu/teclado; arreglar Escape y QColor.
- [x] Render fixture/desconectado; estados en suites QML y contrato real de bridge.

## 4. Presets/voz y despliegue

Files: presets/control/CLI/voice si precisan extensión, servicio/session/build/docs.

- [x] Presets nuevos aditivos, voces cinematic/overlay conservadas; tests RED/GREEN.
- [x] Session launch del componente nativo sin privilegios nuevos.
- [x] Limpiar documentación y separar visor externo experimental del arranque real.
- [x] Registrar matriz de capacidades conectadas/no expuestas y comandos reproducibles.

## 5. Modo claro beta (último)

Files: `Theme.qml`, Apariencia y bindings semánticos de paneles/viewport, tests QML.

- [x] Test RED de selector y tokens de contraste/navegación compartida.
- [x] Añadir paleta clara centralizada y selector beta, preservar oscuro por defecto.
- [x] Render claro/oscuro; contraste, selector y navegación compartida verificados.

## 6. Cierre sin merge

- [x] Pruebas pertinentes, revisión independiente por dominios y fixes comprobados.
- [x] Commits por etapas, push fast-forward al PR41 y bitácora/pendientes actualizados.
- [x] PR sigue Draft/open, main y release intactos. Nunca afirmar GPU/operaciones
  físicas verificadas por fixtures.

## Evidencia y límites del cierre

Ver docs/gui/BITACORA_NATIVE_2026-10-15.md para resultados y pendientes reales.
El código de estas etapas está implementado; instalación Linux/NVIDIA, productores vivos,
audio NeMo y canales gráficos de autoridad pendientes no se acreditan por fixtures.
Los commits nuevos de Xavi 1177407/17a8bee se conservaron al reconciliar la rama.
Publicados por fast-forward hasta 7254666; el PR permanece abierto como Draft.
