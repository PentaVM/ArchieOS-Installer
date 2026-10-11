# Bitácora — ArchieOS-Nvidia

## 2026-10-11 — Claude — Captura en vivo, auditoría continua e impresión

Para Codex: leer `../docs/HANDOFF-2026-10-11-SENTRY-AUDITORIA.md`. El dueño pide que lo
audites y mejores; ese documento lista lo no probado y los defectos que yo mismo veo.

- `archie-netwatch` (crate `archie-lan`): TShark captura, se auditan direcciones,
  puertos, flags y tres nombres por trama, sin payload. Registro por dispositivo y seis
  alertas. Probado en vivo en Linux con tráfico generado: `new-device`, `port-scan` y
  `cleartext-login` salieron; las otras tres reglas sólo tienen prueba unitaria.
- `archie-print` (crate `archie-sentry`): `Print-Job` y `Get-Job-Attributes`, texto a
  páginas JPEG. Probado contra impresora simulada. **Ninguna impresora real**: la
  Brother no estaba en la red. No pasa por `Intent`/journal/aprobación por digest.
- ISO: `build.sh` y `packages.x86_64` los incluyen; sin construir.

Sentry no está terminado: las tres fuentes de red no están unidas ni llegan al world
graph, no hay respuesta, ni adaptador de router, ni baselines.

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

## 2026-10-11 — Codex — Floor de voz y consolidación exclusiva en Nvidia

El dueño corrigió el destino: trabajar siempre en `PentaVM/ArchieOS-Nvidia` para
el hackathon. Claude/Xavi poseen el port WSL y la nueva GUI. El handoff adjunto
describe su encargo; no autoriza a Codex a ejecutarlo. Codex termina voz y mergea
las entregas comprometidas. `AGENTS.md` guarda estas instrucciones para continuar.

### Integración

- Base `0bd6405`; fast-forward de Sentry hasta `aaf2071` (11 commits ya escritos).
- `e5cf04a`: floor/speech-to-speech, interfaz ASR aditiva y voz bilingüe. El commit
  `c9b3603` de la rama de voz ya tiene un patch equivalente en main; se portó su
  trabajo pendiente por archivos conservando la memoria y el wrapper RVC actuales.
- Claude publicó después `b22625e`, WSL Operator: merge `633f7ac`. No se generó,
  importó ni ejecutó la imagen o sus herramientas durante este trabajo.
- `586637d`, auditoría GUI: merge `30978a0`. Son documentos y propuestas, sin código
  GUI ni aprobación de sus ADR. La auditoría se hizo sobre la base antigua; requiere
  actualizar contratos después de esta consolidación.
- Revisión suplementaria de las nuevas entregas: una línea del builder borraba
  `/var/cache/pacman/pkg/*` del host como root. Se retiró en un fix mínimo de
  integración. Auditoría estática de todos sus cleanup top-level: antes rechazaba
  un destino fuera de variables de imagen/workspace; después pasaron los cuatro
  comandos restantes y la sintaxis Bash. No se ejecutó el builder ni se borró esa caché.
- Branches de origin cubiertas por main al cierre. Rama bilingüe histórica equivalente;
  worktrees de otros agentes preservados. El remoto `archieos` se excluye del trabajo.
- El único fallo de bridges informado por la auditoría se reprodujo: índice generado
  GIMP desactualizado. Se regeneró desde manifiestos; cambió una línea, sin alterar
  el bridge ni sus operaciones.

### Voz y revisión

Se conserva ASR/NeMo, SpeechSession/FloorPolicy y la ruta de propuestas/políticas.
El loop retiene finales precoces y cláusulas, evalúa ASR antes de interrumpir, conserva
correcciones y usa recibos del turno correcto. Historial y reproducción ya no se
confunden con texto generado. Cancelación bloquea eventos tardíos y shutdown corta
la cola. Piper ES/EN convive con Magpie/RVC; no se presenta Piper+RVC como compatible.

La revisión final de lectura encontró tres fallos adicionales, reproducidos antes
del fix: acknowledgements repetidos que iniciaban otro turno, corrección corta cuyo
ASR terminaba en silencio, y worker que seguía tras un error TTS. Se corrigieron con
pruebas del loop y del worker. También se mantuvo el override de Piper nativo sin
exigir Python. No se reaplicaron archivos viejos de IRIS, Orbe o memoria.

### Evidencia final

Windows, Rust 1.98.0, lockfile actual; perfiles de test/dev sin símbolos de depuración
para no agotar el disco. Comandos y logs locales en `%TEMP%`:

```text
cargo test --locked -p archie-speech -p archie-voice -- --test-threads=1
155 aprobadas; 0 fallos; 0 omitidas; salida 0
archie-speech-review-final.log

cargo test --locked -p archie-core -p archie-cli -p archie-sentry -p archie-lan -p archie-state -p archied -p archie-workstation -- --test-threads=1
318 aprobadas; 0 fallos; 84 omitidas; salida 0
archie-nvidia-affected-final.log

cargo test --locked -p archie-bridges --lib -- --test-threads=1
55 aprobadas; 0 fallos; 0 omitidas; salida 0
archie-bridges-final.log
```

Total: 528 aprobadas, 0 fallos, 84 omitidas. Omitidas: 66 requieren el cluster Core,
16 un fixture PostgreSQL 18 aislado, 1 una fase de crash recovery aislada y 1 FreeCAD
empaquetado para Arch Linux. PostgreSQL 18 existe, pero faltan los binarios/control
de TimescaleDB 2.29.2 y pgvector 0.8.6 y no se configuró otro cluster. No se conectó
a bases existentes ni se instalaron extensiones para simular esa validación.

Pasaron sintaxis Bash de siete scripts (tres de voz y cuatro WSL), AST Python del
bridge y AST PowerShell del installer. Los scripts WSL solo se analizaron, no se
ejecutaron. `git diff --check` pasó. Se verificaron configs/cards Piper remotos y
sus MD5/sample rates; no se descargaron los pesos ni se sintetizó audio real.

La compilación amplia inicial se interrumpió por poco espacio. La limpieza recursiva
fue rechazada por revisión automática; `cargo clean --target-dir` limpió 2.5 GiB de
artefactos y permitió verificar con debug=0. No se borraron fuentes o worktrees.

### Relevo

Leer `../docs/HANDOFF-2026-10-11-SPEECH-FLOOR.md`: incluye benchmarks reproducibles,
límites de pacing sin ACK del navegador, bloqueo del sink, ASR heurístico y pruebas
native `speech/live/rvc` pendientes del SDK/modelos. No hay cifra TTFT ni latencia
audible medida para Spark/Halo/Mac mini/GPUs antiguas ni convivencia con Steam.

Operator no empaqueta esta voz todavía ni Observatory. Tiene automount activado y
interop desactivado; su acceso al host y IRIS en esa imagen no se validaron aquí.
Claude debe conectar la entrega de voz a su packaging y Xavi probar el entorno real.
Los pendientes de Sentry siguen en su auditoría: integración no equivale a pairing,
WorldGraph/pilares completos, instalación de workers o dispositivos verificados.

Push autorizado a origin/main, normal y sin force. Comparar HEAD con el remoto antes
de avisar; este registro forma parte de la publicación. No volver al repo histórico.
