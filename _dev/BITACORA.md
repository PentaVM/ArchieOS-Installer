# Bitácora — ArchieOS-Nvidia

## 2026-10-11 — Claude — Un solo repositorio, Sentry portado, LAN acotado

Decisión del dueño: **solo se usa `PentaVM/ArchieOS-Nvidia`**. `PentaVM/ArchieOS` queda
como historia; no continuar trabajo allí. Lo que sigue reemplaza la tabla de dos
repositorios de `../docs/HANDOFF-2026-10-11-SENTRY-CONTEXTO.md`.

### Portado desde ArchieOS

Los seis commits de Sentry (`5012a14`, `5423249`, `c950db0`, `999f355`, `34eaa19`,
`0fabaff`) entraron por cherry-pick con `-x`, conservando autoría y mensaje: crate
`archie-sentry`, `sentry`/`sentry_config`/`operator`/`operator_session`/`passive_network`
en Core, comandos del CLI y cambios de `archie-state` y `archied`. Los únicos conflictos
fueron el workspace, `Cargo.lock` (regenerado) y un brazo del `match` del CLI, donde se
conservan `SentryNetworkAnalyze` y el `local_command` de Workstation. También
`docs/SENTRY-HOGAR-RED-VOZ.md` y `docs/TODO-COMPATIBILIDAD-MULTIPLATAFORMA.md`. No se
trajeron la copia de `archie-lan` (el original ya vive aquí) ni los planes de 2026-10-02.

### Revisión LAN cerrada

Los cuatro puntos de la entrada anterior: plazo total de 6 s por respuesta HTTP, `LanUrl`
solo construible por su parser, topes de respuestas SSDP, hosts y servicios mDNS y 45 s
para consultar a todos los dispositivos, inventario 0600. `build.sh` compila e instala
`archie-lan` y `archie-rem` (unidad de REM instalada, no habilitada).

### Workstation

Un adaptador Enterprise sin dominio veía las tareas de todas las organizaciones; ahora
solo ve las que no llevan dominio. `POST /sync` responde 501 mientras no haya transporte.

### Verificación

Windows, un crate a la vez: `archie-sentry` 35, `archie-state` 10, `archie-core` 126,
`archie-cli` 19, `archied` 65, `archie-workstation` 28, `archie-lan` 19; 0 fallos. Los
omitidos piden PostgreSQL aislado o FreeCAD. Linux: ver el PR. No se construyó ISO.

### Qué no existe todavía

Captura de red (dumpcap/TShark en vivo), auditoría continua, alertas, DNS/flujos y
respuesta: nada de eso está hecho. `sentry-network-analyze` lee un archivo ya exportado
y `archie-lan` pregunta por SSDP/mDNS; ninguno de los dos alimenta al otro ni al world
graph, y nada produce un `PillarContext`. `PillarContext` gradúa todo `restricted`, que
no sale a la nube: en un nodo sin modelo local esos pases terminan en rechazo.

## 2026-10-11 — Codex — Integración y relevo para Claude Code

El dueño autorizó commits, merge y push a main. Leer primero
`../docs/HANDOFF-2026-10-11-SENTRY-CONTEXTO.md` y `HANDOFF.md`.

### Contexto e invariantes

La interfaz principal es voz. Conservar Core, Cortex/Cerebra, IRIS, Policy,
leases, scheduler y autoridad determinista. El host aporta recursos de inferencia;
no crear un segundo Cortex en Windows. Mantener uso general del PC y compatibilidad
amplia con hardware consumer, Mac mini, Spark, Halo y GPUs antiguas, con evidencia
por plataforma. Prioridad actual: Sentry, IoT, routers y análisis de red defensivo.
Conectar Wanda, Vault, REM y Forge a los loops existentes y separar vida personal
de trabajo, conservando el bridge Enterprise de Santiago Rearte.

### Integración realizada

- Main actualizado a `653fc41` antes de integrar nuestro trabajo. Se preservan
  reasoning/REM (`d49bc92`), LAN (`3e89187`, PR #26) y Veil (`f925944`, PR #27).
- `da3eb6f` guarda la separación de dominios personal/laboral en Vault,
  `PillarContext` y su paso por Cerebra, y los cambios Enterprise/BoardAPI.
- `18613ba` fusiona `feat/unify` en main. Vault exporta dominio y Veil. Los cambios
  de contexto se compararon con su fuente; no se sustituyó la librería actual
  por una versión antigua ni se reescribió IRIS.
- El otro repositorio, `PentaVM/ArchieOS`, recibió Sentry operativo, consulta HA,
  análisis pasivo TShark (`0fabaff`) y el port LAN (`e93821f`). Son dos árboles
  distintos; este turno no los fusiona completos.

### Revisión del LAN de Claude

`archie-lan scan/watch` descubre SSDP/mDNS, consulta descripciones UPnP del host
anunciante y estado Roku. Sus 18 tests pasan. El run router/Roku reportado por
Claude no se repitió aquí. Clasificación heurística no es pairing ni identidad.
El inventario todavía no entra al world graph.

Pendientes comprobados por lectura del código: compilar/instalar el binario desde
`distro/archiso/build.sh`, provisionar salida y probar permisos reales del worker;
deadline HTTP total y validación de `LanUrl` al entrar a I/O; límites agregados
de respuestas, hosts, servicios y duración; retención y estado parcial del inventario.
No se corrigieron ni se presentan como fallos explotados en esta consolidación.

### Verificación final

Windows, Rust 1.98.0. Después del merge real en main:

```text
cargo test --locked -p archie-vault -p archie-cerebra -p archie-workstation -p archie-rem -p archie-lan -p archie-model-runtime -- --test-threads=1
179 aprobadas; 0 fallos; 3 omitidas; salida 0
```

Log local: `%TEMP%/archie-context-final-main-validation.log`.
ArchieOS main: Core/CLI/Sentry/LAN, 196 aprobadas, 0 fallos, 27 omitidas,
salida 0; log `%TEMP%/archie-main-final-validation.log`.
Los omitidos requieren sus fixtures/servicios; no equivalen a verificaciones
de PostgreSQL aislado, FreeCAD instalado, Linux, ISO, MSRV, dispositivos,
biometría real ni rendimiento de juegos. `git diff --check` también pasó.

### Dónde continuar y qué queda

Continuar en este main:
`C:/Users/Penta/.codex/worktrees/archie-nvidia-main-integration`.
El worktree fuente `C:/Users/Penta/Desktop/ArchieOS-nvidia-unify` queda preservado
con un cambio antiguo de Cargo.lock de dependencias ya presentes en upstream.
No copiarlo a main ni reaplicar diffs históricos sin revisar.

Cerrar empaquetado/límites LAN y conectar productores Sentry con PillarContext,
Cerebra, Forge, resultado verificado y Vault/REM. Añadir adaptador del router
según firmware real, telemetría defensiva, voz y cancelación. Lector biométrico,
conectores de nodos/móviles y app iOS siguen pendientes. El handoff compartido
detalla contratos y orden de trabajo; no atribuir al código capacidades completas
de Jarvis ni declarar entrega de mensajes desde stubs.

Publicación: push normal de cada main, sin force, y comparación de HEAD con
`git ls-remote origin refs/heads/main` antes de avisar. El SHA final se consulta
en Git; este registro forma parte del commit que se publica.
