# docs/gui — GUI profesional (Quickshell + Vault)

Trabajo en curso en la rama `feat/professional-gui-quickshell-vault`. Orden de lectura y estado:

| Documento | Fase | Estado |
|---|---|---|
| [AUDIT_CODEBASE.md](AUDIT_CODEBASE.md) | A | escrito |
| [CAPABILITY_REGISTRY.csv](CAPABILITY_REGISTRY.csv) | A | 114 capacidades canónicas, ruta y símbolo verificados |
| [CAPABILITY_SYMBOL_INDEX.csv](CAPABILITY_SYMBOL_INDEX.csv) | A | 3 841 símbolos públicos asignados a una capacidad canónica |
| [GUI_FUNCTION_BACKLOG.md](GUI_FUNCTION_BACKLOG.md) | A | 88 ítems `GUI-*` + 24 `GUI-VAULT-*` |
| [ARCHITECTURE_GUI.md](ARCHITECTURE_GUI.md) | A | ADR propuestos, sin aceptar |
| [INTEGRATION_MATRIX.md](INTEGRATION_MATRIX.md) | A/C | 2 `INTEGRATED`, 2 `IN_PROGRESS`, 0 `TESTED` |
| [TEST_REPORT.md](TEST_REPORT.md) | A | línea base |
| [DESIGN_SYSTEM.md](DESIGN_SYSTEM.md), [SCREEN_SPECS.md](SCREEN_SPECS.md), [MOTION_CINEMATICS.md](MOTION_CINEMATICS.md) | B | escritos; capturas en `screenshots/`; prototipo en `display/shell/prototype/` |

Los CSV se generaron con un script de auditoría que falla si una ruta o un símbolo no existe en el árbol; no se editan a mano sin repetir esa comprobación.

Fase C (en curso): crate `core/crates/archie-shell-bridge` (adaptador de sólo lectura) y su consumo en
`display/shell/prototype` (`BridgeModel`, `LinkChip`, `ResourceGauges`); host Quickshell sin probar en
`display/shell/quickshell`.
