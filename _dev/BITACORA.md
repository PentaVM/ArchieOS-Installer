# Bitácora — ArchieOS-Nvidia

## 2026-10-29 — Claude — ArchieOS-Setup.exe: un solo archivo para instalar en Windows

Pedido del dueño: en la web de descargas, solo un exe (el launcher compilado, que la primera vez
instala), sin zip. `distro/windows/setup/ArchieOSSetup.cs` (C# 5, .NET Framework 4) lleva dentro
el launcher, su motor, los textos, el ícono de la marca y el manifiesto firmado
(`setup/payload.txt`). La primera vez corre `Install-ArchieOS-Launcher.ps1` con una ventanita de
progreso y abre el launcher en Instalar; queda copiado como `Launcher\ArchieOS.exe` y los accesos
directos lo abren. Otra compilación (otra descarga) se instala encima; un archivo que falta se
repone igual. `--install` instala sin ventana; el log queda en `%TEMP%\ArchieOS-Setup.log`.
Manifiesto `asInvoker`: Windows no pide administrador aunque el nombre diga Setup.

- **Bug arreglado** (también estaba en el zip 0.1.0): `Install-ArchieOS-Launcher.ps1` cerraba todo
  PowerShell cuyo comando contenía `ArchieOS-Launcher.ps1`, y su propio nombre lo contiene: se
  mataba a sí mismo. Lo mismo `Uninstall-ArchieOS-Launcher.ps1`. Ahora excluyen su `$PID` y
  piden la ruta exacta del launcher.
- **Construcción**: `setup/Build-ArchieOS-Setup.ps1` con el `csc.exe` de Windows;
  `setup/build.sh` en Linux con el Roslyn del SDK de .NET y los ensamblados de referencia de
  .NET 4.8 (determinista). `New-Release.ps1` arma el exe en lugar del zip.
- **Verificado aquí (Linux)**: compila con `-langversion:5`; PE32 AnyCPU (corre en 64 bits),
  manifiesto `asInvoker`, ícono de 7 tamaños, recurso de versión 0.1.0; los 13 archivos que lleva
  son byte a byte los del repo; `setup/test-setup-exe.sh` corre el exe en .NET 8 contra un
  PowerShell falso: instalación, reapertura con argumentos, otra compilación, archivo faltante y
  fallo con su motivo en el log.
- **Sin verificar**: nada en Windows. Ni la ventana, ni el script real, ni los accesos directos,
  ni Configuración → Aplicaciones, ni SmartScreen (el exe no está firmado con código: Windows
  avisa «Windows protegió tu PC» → Más información → Ejecutar de todas formas).

## 2026-10-21 — Claude — IRIS C2–C10, archie-accel, S-LINKs, ResourceAdvice, slots del LLM

Pedido del dueño: implementar todos los C y las arquitecturas del plan
(`_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`, en `perf/wanda-runtime`). Registro completo,
con qué se verificó y qué no: `IMPLEMENTACION-IRIS-ACCEL-SLINKS-2026-10-21.md`.

- **IRIS**: C3 (carga perezosa del zip, torre de texto en CPU, caché de frases en disco; arreglado
  un bug previo de offsets de storage), C5–C10, C2 detrás de `flash-attn`. `tests/weights.rs`
  sobre un `.pt` con la forma real escrito sin PyTorch; `tests/pipeline.rs` corre todo el
  runtime sobre un modelo aleatorio con las formas de la release.
- **F3**: `core/crates/archie-accel` (daño, seguimiento, plan, VRAM y residencia de expertos,
  `ResourceAdvice` con sobre de Policy). En IRIS: `/live` (cada cuadro respondido sin esperar al
  modelo; lecturas en un worker), `detect_many` con capacidad del MoE por imagen, residencia de
  expertos (apagada), histogramas de ruteo.
- **F4**: `core/crates/archie-slink` (en proceso y entre procesos por memfd+seqlock+eventfd, 8
  enlaces tipados con grado de privacidad, joiner, puente de texto etapa A). IRIS publica y
  exporta `scene.state`.
- **Cortex**: `resources.hpp` (`ResourceAdvice` como `CandidateSchedulerChange`, `focus.plan` con
  fixture compartido C++/Rust, descriptores de capacidad).
- **§9**: `archie-model-runtime::llama_slots` (prefijo estable, slots en SSD de `llama-server`
  local).
- **Herramientas**: `archie-iris-bench` (F0) y `scripts/iris/verify-rtx3070.sh`; la paridad
  acepta `IRIS_DEVICE=cuda`.
- **Verificado aquí**: pruebas de CPU; build y clippy con `--features cuda` y `flash-attn`
  (nvcc 12.8, `CUDA_COMPUTE_CAP=86`); clippy con Rust 1.88 y 1.97; CMake/ctest de Cortex.
  Seguimiento: 20 cajas a 640×480 en 3.9 ms por cuadro (4 hilos del contenedor).
- **Sin verificar**: cualquier tiempo o VRAM en la 3070, la paridad con los pesos reales (en
  CPU y en GPU), flash-attention en ejecución. La inferencia completa por cuadro no llega a
  60 fps en la 3070 por las cuentas del plan; la rama responde cada cuadro a la tasa de la cámara
  y lee con el modelo tan seguido como dé la GPU, y el bench mide las dos cosas.
- **No hecho**: `read_decoder` (falta el código de Foundry), el lado de Wanda (vive en
  `perf/wanda-runtime`; un merge de prueba choca con el veil del #35 en `archie-voice`), F5 y
  F6, y el entrenamiento, que sigue en la WSL del dueño.

## 2026-10-21 — Claude — IRIS F1: C4 y C1 (frases en lote, MoE sin lecturas al host)

Sigue el paso 3 de `HANDOFF-WANDA-IRIS-2026-10-20.md` y §3/§12 de
`_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md` (rama `perf/wanda-runtime`).

- **C4** (`siglip.rs`): `.contiguous()` sobre la última posición de la torre de texto. Es el
  fallo de CUDA "matmul is only supported for contiguous tensors" con 2 o más frases nuevas.
  **No reproducido ni verificado en GPU**: en CPU candle acepta la vista con stride.
- **C1** (`model.rs`, `SparseMoe`): dos rutas con el mismo resultado.
  - `route_on_device` (GPU): sin `to_vec1`. Top-k por máximos repetidos (empates al experto
    menor, como el sort estable), posición en la cola por conteos acumulados por bloques de
    128 (el `cumsum` de candle multiplica por un triángulo N×N, 4.8 GB en modo abierto),
    productos por lotes de 8 expertos sobre `[E, capacidad, D]`, y cada token suma sus
    expertos en orden ascendente, como el bucle anterior.
  - `route_on_host` (CPU): el algoritmo anterior, sin cambios. Medido en este contenedor
    (CPU de 4 núcleos, forma real 512→2048, 32 expertos): la ruta por lotes tardó 722 ms
    contra 539 ms con 1 fila y 6.3 s contra 4.0 s con 17 filas, porque multiplica las ranuras
    vacías de la capacidad 1.5×. Por eso la CPU conserva la ruta del host.
  - La capacidad sigue siendo una sola para todo el lote, como en el entrenamiento. Calcularla
    por imagen en lotes con varios focos (handoff §4.2) cambia resultados y es de F3.
- **Verificado** (CPU, f32): `device_routing_matches_host_routing` compara las dos rutas con
  8 y 12 expertos, top-2 y top-3, empates y desbordes, y un escaneo de tres niveles. Con
  umbral 0 dieron idénticas bit a bit; la prueba deja 1e-5 por si otra CPU elige otro kernel
  de gemm. `tokens_past_capacity_keep_only_the_shared_expert` comprueba los descartes.
  `cargo clippy -p archie-iris-runtime --all-targets -- -D warnings` y `cargo fmt` limpios.
- **Arreglo de revisión** (`9fc97b9`, del dueño): en la release los sesgos de los expertos son
  `[E, 1, H]`; `route_on_device` les agregaba otra dimensión y el matmul por lotes fallaba, así
  que el camino de GPU no corría con los pesos reales. Ahora se llevan a `[count, 1, H]` con
  cualquiera de las dos formas, y las pruebas arman los sesgos como la release. Sin el arreglo
  fallan 2 de las 3 pruebas del MoE; con él pasan.
- **Sin verificar**: la build `--features cuda` (sin toolkit aquí), la paridad con
  `tests/parity.rs` sobre los pesos reales y cualquier latencia en la RTX 3070. La meta de F1
  (open ≤ 300 ms, focused ≤ 90 ms) sigue sin medir.

## 2026-10-12 — Claude — IRIS en Rust, memoria y Wanda probadas

- **Runtime de IRIS sin Python** (`core/crates/archie-iris-runtime`, binario
  `archie-iris-serve`).
  - Lee `iris-beta21-step132000.pt` (SHA-256 verificado) con el lector de pickles de candle,
    sin ejecutar código.
  - Porta V-JEPA 2.1 desde `facebookresearch/vjepa2@204698b`, SigLIP2 (visión y texto), la
    fusión MoE de 32 expertos, el decodificador, las incrustaciones de instancia y la
    cabeza de acción con su memoria por sesión.
  - Sirve la API de `iris_serve`.
- **Paridad medida contra el IRIS de Python**, en CPU float32, con los pesos reales y una
  captura de 1920×1080:
  - Canvas idéntico byte a byte.
  - Error relativo de 1e-4 en los encoders y de 1e-6 en la fusión.
  - 104/104 y 60/60 detecciones idénticas; 3/3 frames de `/act` iguales.
  - La torre de texto, contra `transformers` con pesos aleatorios: 1e-6.
  - Las frases fueron features sintéticas porque la torre real no viene en la release y
    Hugging Face está bloqueado aquí: las detecciones no significan nada, solo coinciden.
- **Velocidad en CPU** (4 núcleos): encoders en 3,8 s contra 1,4 s de PyTorch. El build
  `cuda` no se compiló ni se corrió: no hay GPU ni toolkit.
- **Memoria personal y Wanda**:
  - `a_person_named_aloud_is_remembered_as_what_the_speaker_said` (`archie-voice`): «Pedro
    es mi hermano» sobrevive 40 días y un reinicio como cita atribuida y no verificada.
  - `sentry/tests/test_wanda_memory.cpp`, 27 comprobaciones: la persona persiste en el
    grafo y Wanda propone `Friend` como `Inferred` con 630‰.
  - La hipótesis solo se guarda como arista `ModelHypothesis` con la nota «wanda»; se
    pierden la confianza, el falsificador y la expiración.
  - Al confirmarla Pedro, la arista pasa a `SubjectDeclared` con Wanda como nota.
  - No hay un puente de la memoria de voz al grafo de Wanda, ni un productor de
    `CoPresence` desde la cámara.
- **Arreglo menor**: `sentry/src/cli/main.cpp` no compilaba con GCC 13 por un
  `#include <array>` que faltaba. Ahora pasan 17/17 pruebas de Sentry.
- **Kimi se quedó sin créditos** a mitad de su ola 3. Se abrió la sesión
  `session_01YNCXAzdiH83GtVLw6Dh3Y2`, que parte de `todo/pendientes` (`6a977c4`), para
  Chromecast CAST v2, el feeder con focos y los servidores de protocolo en proceso.
  Empuja a `claude/sentry-ola3-cast-focos-servidores`.

## 2026-10-11 — Claude — Solo `main`, pendientes en un PR, Kdenlive en la imagen

Codex se quedó sin créditos a mitad de la consolidación; esto la cierra.

- **Fusionado**: el prototipo de GUI de Xavi (`4b282cd`, PR #31). Codex ya había
  fusionado los PR #28, #29 y #30 y la voz (`e5cf04a`).
- **Ramas**: borradas las doce ramas remotas; cada una se comprobó contenida en `main`
  antes de borrarla. Queda `main` y la rama del PR de pendientes.
- **Pendientes**: `../docs/PENDIENTES.md` es la lista única, por área y con dueño. El PR
  en borrador `todo/pendientes` la lleva como lista de tareas.
- **Operador WSL**: `archie-operate --app kdenlive`, `archie-session run PROGRAMA`, la
  política de Kdenlive, y `build-operator.sh` pide `kdenlive mlt ffmpeg` y los binarios
  `archie-bridges` y `archie-mcp-server`. **Solo revisado por sintaxis**: no se
  construyó imagen (2 GB libres en el constructor) ni se abrió Kdenlive en el Stage.
- **Pre-release `wsl-operator-2026.10.11`**: no contiene todo. Salió de `b22625e`, antes
  de la voz y de Kdenlive. Ver la sección 1 de los pendientes.

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
- Branches terminadas de origin cubiertas por main. En el último fetch llegó
  `4b282cd`, prototipo GUI Fase B en curso: se deja en la rama de Claude, sin tratarlo
  como integración funcional. Rama bilingüe histórica equivalente; worktrees de otros
  agentes preservados. El remoto `archieos` se excluye del trabajo.
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

Publicación observada: push normal y remoto comprobado en
`b0e2e77eac8afefc1606917be55a8add11cb724a`, working tree limpio. El commit posterior
solo registra esta evidencia y el estado del plan; también se publica y se compara
su HEAD con origin/main antes de la respuesta final.

## 2026-10-12 — Sentry, ola 3 (Claude, relevo de Kimi K3)

### Contexto

Kimi K3 se quedó sin créditos a mitad de ola. Rama `claude/sentry-ola3-cast-focos-servidores`
desde `todo/pendientes` (`6a977c4`, 8 commits de Kimi fuera de `main`; el PR #32 ya se
había fusionado en `28d03f8`). Lo que Kimi tenía sin commitear se perdió y no se intentó
reconstruir. Se fusionó `origin/main` (`ac9c1d2`: voz, Bonsai 4B, bridge del shell) con
merge, sin conflictos.

### Verificación del trabajo heredado

Kimi informó 484 pruebas verdes en Windows. En Linux, tras la fusión, fueron 463
aprobadas y **28 fallidas**: `adapters_host` (18) y `adapters_router` (10) usaban rutas
`C:/…` para ssh y la clave, que no son absolutas fuera de Windows, y `SshConfig` las
rechaza. Se corrigieron los fixtures, no la validación (`0970b1b`).

### Lo hecho

1. **Chromecast CAST v2 real** (`5551856`): TLS 8009 con `rustls`/`tokio-rustls`
   (ring, ya en el lockfile por reqwest) y `prost` (derive, sin `protoc`). Tramas con
   prefijo de longitud, CONNECT/CLOSE, PING/PONG, GET_STATUS, LAUNCH, STOP, SET_VOLUME.
   Contratos nuevos `AppStop` y `MuteSet`, estado `ResourceState::Cast`. Se verifica:
   SHA-256 del certificado hoja igual al pin del operador y firma del handshake con su
   clave. No se verifica: cadena, nombre, fechas ni `tp.deviceauth`. Prueba de bytes
   protobuf escritos a mano para no validar el códec contra sí mismo.
2. **Feeder unificado con perfiles de foco** (`84efe05`): `archie_cerebra::{feeder,focus}`,
   `archie focus <perfil>`, `docs/SENTRY-FOCO.md`. Los umbrales de Wanda se calibraron a
   su escala real (un «urgente» pesa unos 12), no a la supuesta.
3. **Servidores de protocolo con estado** (`98a3c7e`, `8fd6788`): ver README del crate.
   Defectos encontrados y corregidos: Roku en la pantalla de inicio y carrera del informe
   MQTT tras `apply`.

### Resultados

```
cargo test --locked --no-fail-fast -p archie-sentry -p archie-cerebra -p archie-core -p archie-cli -p archie-workstation
530 aprobadas; 0 fallos; 37 omitidas (PostgreSQL aislado y hardware)
```

`cargo fmt` (rustfmt de los archivos nuevos) y `clippy --tests` sin avisos en lo propio;
los avisos que quedan en `archie-vault`, `contracts.rs` y `archie-cerebra` son anteriores.
Humo con el binario real: `archie wanda-ingest` de un volcado de notificaciones y luego
`archie focus personal` → 3 referencias admitidas, `refused:NoLocalModel`.

**Nada se probó contra hardware real**: ni Chromecast, ni Roku, ni impresora, ni router,
ni Matter. Las pruebas nuevas son de protocolo en loopback. A mitad de la verificación se
agotó el disco (22 GiB de `target`); `cargo clean` y se repitió completa.

### Relevo

- Probar `archie sentry cast-probe` y una acción contra un Chromecast real; medir si la
  rotación del certificado ocurre y cada cuánto.
- Autenticación de dispositivo Cast (`tp.deviceauth`) si se quiere probar «Google
  genuino» y no sólo «el mismo equipo».
- Los perfiles de foco son criterio; ajustarlos con uso real del dueño.
- No se tocó la GUI Quickshell/Vault, la generación WSL ni `archie-iris-runtime`.

## 2026-10-12 — Día autónomo (Claude)

Nelson pidió trabajar solo durante el día: expandir el dominio y llevarlo hacia
producción. La cola y el protocolo están en `_dev/AUTONOMO.md`. Una rutina horaria
(`trig_01WLVcpbjBBHdU3DqZjsvKxK`, minuto 38) despierta esta sesión; se disparó a mano
una vez para comprobar que entrega.

### CI del PR #36

Estaba rojo también en `main`:

- `cargo fmt --check`: 130 archivos sin formato.
- clippy `-D warnings` con 1.88: unos 30 avisos en orb, paint, arena, displayd, voice,
  lan, desk, skill-node, types, sentry y cli.
- `native-contracts`: faltaba `#include <array>` en `sentry/src/cli/main.cpp`.
- Dos pruebas rotas:
  - `archie-model-runtime`: `Content-Length: 32` para un cuerpo de 30 bytes.
  - `archie-observability`: `0::/` (cgroup raíz) se trataba como ruta inválida y no
    como `Unsupported`.
- `cargo test --workspace` se quedaba sin disco en el runner. Se resolvió con
  `CARGO_PROFILE_*_DEBUG=0` en el job: el `target` baja de unos 30 GiB a 5 GiB y las
  pruebas no cambian.

Local, con 1.88: fmt y clippy limpios; `cargo test --workspace`, 2329 aprobadas antes de
las dos correcciones y 0 fallos después en los crates tocados; `ctest` de cortex,
sentry, visual y osm, 40/40.

### Dominio

- **IPv6** (`e41b83d`): `address-moved` también por Neighbor Advertisement.
- **Avisos al dueño** (`f16aef5`): crate `archie-notices`, follower, `archie
  sentry-notices`, `GET /api/notices` y `[sentry.response]`. Lock exclusivo entre
  escritores: 8 hilos, 80 avisos, ninguno perdido. Frase hablable sin datos de la red.
  Ver `docs/SENTRY-RESPUESTA.md`.
- **`traffic-surge`** (`b1501fc`): baseline de volumen por dispositivo. Umbrales de
  criterio; sin tasa de falsos positivos medida.
- **Unidad de netwatch** (`1154eca`): como `DynamicUser` no podía funcionar en Arch,
  porque `dumpcap` es `root:wireshark` 0754 y el estado quedaba en `/var/lib/private`.
- **Follower**: sin `alerts.jsonl` moría (`0e05975`); ahora tiene unidad propia
  (`01e61e3`).

Humo con el binario real: `sentry-follow --once` sobre dos alertas sintéticas publica 1
aviso (el `new-device` queda por debajo del umbral); `sentry-notices` lista, acusa por
prefijo y rechaza un id que no existe con salida 1. El archivo queda 0600 en esa prueba,
y 0640 desde `01e61e3`.

**Nada se probó en hardware ni en una red real.** El modo `live` de `archie-voice` no
compila aquí (necesita el SDK de NeMo): la voz no dice los avisos todavía, y el contrato
está en `docs/SENTRY-RESPUESTA.md`.

### Continuación (Claude, 2026-10-12, 11:20–11:50 UTC)

- **CI:** verde completo en GitHub desde `4625bb7` (`rust` + `native-contracts`). El
  paso nuevo de TShark e2e pasa en el runner (`5c9e47b`).
- **Inestable en el CI, con causa raíz** (`8977076`): el nonce WS-Security de ONVIF se
  repetía dentro de un milisegundo y el servidor de prueba lo rechazaba como reutilizado,
  igual que haría una cámara real. Ahora lleva un contador de proceso.
- **Auditoría de E/S de los adaptadores** (`705a38f`), cada arreglo con una prueba que
  falla sin él:
  - panic por cortar por bytes un stderr no ASCII; con `panic=abort` mataba el proceso;
  - ADB, BLE, Matter y WoL bloqueaban un hilo de Tokio y el plazo del Intent no podía
    aplicarse;
  - tope de sesiones `loginctl`.
- **e2e con TShark real** (`5c9e47b`): una captura construida byte a byte pasa por
  `archie-netwatch read`.
- **`unusual-destination`** (`88670e7`).
- **Productor del pilar Vault** (`531c860`): `vault_bridge`, `archie vault-docs` y la
  integración con el feeder.
- **`sentry-notices --claim`** (`f7f7572`): una sola llamada para que la voz diga los
  avisos.
- **Relevo:** `docs/HANDOFF-2026-10-12-SENTRY-OLA3.md`, con la validación en hardware
  paso a paso y las decisiones que son de Nelson.

Humo con binarios reales: pcap → netwatch → follower → tablón → displayd
`/api/notices`; `vault-docs` → `focus trabajo`; `--claim` tres veces y luego `null`.

## 2026-10-15 — Bloqueo de dispositivos como Intent (Claude)

Pedido de Nelson: implementar el bloqueo como `Intent` y revisar los otros PR sin
mergear los de XaviCottier.

- **Contrato:** `Command::DeviceBlock { mac, blocked }`, `CommandKind::DeviceBlock`,
  `ResourceState::Router.device_blocked` (`serde(default)`: recibos viejos siguen
  leyéndose), `validate_mac` (una sola dirección de equipo, minúsculas).
- **Router (SSH):** `block_rule` arma el argv exacto (`-C`/`-I`/`-D`, herramienta
  conocida, MAC válida; nada más llega al router). Cortafuegos detectados con `-S
  FORWARD` en el probe o a la primera, de solo lectura. Inserta solo si falta; borra
  hasta que `-C` diga que no queda. `protected_macs` en `[sentry.router]`.
- **CLI:** `archie sentry-node block|unblock <router> <mac>` → `SentryPrepare`, misma
  revisión y aprobación por digest que el resto.
- **Pruebas:** cortafuegos simulado con estado (inserción en ambos, idempotencia,
  duplicados, MAC ajena intacta, protegidas y malformadas sin procesos, sin
  cortafuegos = `Unsupported`, ssh 255 = `Unavailable`, desacuerdo = no verifica),
  configuración (`protected_macs`, router solo IGD sin la capacidad), parseo CLI.
- **No verificado en hardware:** el firmware del MitraStar puede no traer el módulo
  `mac` de iptables; entonces responde `Unavailable`.
- **Otros PR (solo revisión, nada mergeado):** #35, #38, #39 limpios contra `main` y
  contra este cambio, CI verde. #40 (Xavi) CI rojo por `rustfmt` en
  `archie-cli/src/visual.rs`. #41 (Xavi) en conflicto con `main` (displayd, orb lumen,
  voice, `build.sh`) y sin CI. #35 y #39 solo chocan entre sí en `Cargo.lock`.
