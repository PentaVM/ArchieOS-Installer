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
