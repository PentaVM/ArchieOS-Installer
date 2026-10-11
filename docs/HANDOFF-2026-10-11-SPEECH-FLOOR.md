# Voz y consolidación de ramas — 2026-10-11

Continuar exclusivamente en `PentaVM/ArchieOS-Nvidia`. El dueño autorizó merge y
push a main. El handoff profesional Quickshell/Vault sirve como contexto del trabajo
de Claude/Xavi: no se implementa aquí esa GUI ni se genera/modifica su WSL.

## Integración y procedencia

- Base de este turno: `0bd6405`. Fast-forward de `feat/sentry-complete` a `aaf2071`:
  11 commits de Sentry operativo, límites LAN, netwatch y empaquetado IPP existentes.
- Los otros heads locales son ancestros de main, salvo `codex/bilingual-piper-voice`
  (`c9b3603`). `git cherry main codex/bilingual-piper-voice` lo marca equivalente:
  memoria persistente ya está implementada en main. No fusionar su archivo antiguo
  sobre la memoria actual: perdería los eventos retenidos incorporados después.
- El diff bilingüe pendiente de `ArchieOS-nvidia-speech` se porta por archivos,
  adaptando su CLI a la conversión RVC actual. Su worktree original queda intacto.
- Se preservan los cambios de render/Orbe del checkout principal, el Cargo.lock del
  antiguo worktree unify y los scripts/rootfs WSL staged por Claude en su worktree.
- `AGENTS.md` registra repositorio, autoridades y responsabilidades vigentes.
- Durante la validación Claude publicó `b22625e` (WSL Operator), integrado mediante
  `633f7ac`, y `586637d` (auditoría GUI), integrado mediante `30978a0`. Son sus
  entregas existentes: no se escribió una GUI ni se construyó/importó una imagen.
  Los ADR de `docs/gui` siguen propuestos, sin aceptar, y la auditoría registra la
  base anterior `0bd6405`: Claude debe actualizar sus referencias después del merge.
- Revisión de integración retiró una línea del build Operator que borraba la caché
  pacman del host. La limpieza del rootfs se mantiene. La comprobación estática de
  destinos de limpieza pasó después del fix; no se ejecutó el build para probarlo.
- Último fetch: `4b282cd` añade un prototipo GUI Fase B. No se importa ese trabajo
  en curso de Claude/Xavi ni se lo declara integración funcional. Main guarda la
  auditoría Fase A; la continuación de GUI permanece en su rama propia.

## Comportamiento de voz corregido

El ASR sigue siendo el existente; `asr.rs` añade una interfaz a su ABI para probar
el mismo loop de producción sin instalar otro reconocedor. FloorPolicy y
SpeechSession conservan la decisión de responder y la frontera de propuestas.

Un final de ASR ya no inventa 800 ms de silencio. Se retiene el texto hasta que el
audio y la política permitan responder; dos endpoints de una cláusula incompleta
se unen, con límite de 16 KiB. La evidencia de reconocimiento se procesa antes
de decidir una interrupción. Un acknowledgement no cancela la respuesta; una
corrección durante reproducción invalida el turno antiguo y puede recibir respuesta.

Los recibos llevan identidad del turno y secuencia. Un recibo tardío no marca texto
de un turno nuevo como escuchado. El historial espera al resultado del sink en vez
de registrar todo el texto sintetizado. Un fallo TTS termina el loop y cancela la
cola; shutdown cancela la generación activa. Eventos y propuestas tardías de un
backend cancelado quedan bloqueados. Probabilidades deserializadas inválidas se
rechazan y una configuración de thresholds inválida mantiene el floor en silencio.

Piper puede mantener procesos separados para español e inglés, con detección
heurística por frase y fallback español si no hay modelo inglés. Se rechaza un
par con sample rates distintos. Live admite Piper; Magpie y su wrapper RVC siguen
existiendo. Piper+RVC se rechaza explícitamente: no se presenta como soportado.
El bridge Python es un adaptador TTS externo, no otro Cortex ni otro planner.

## Pruebas reproducibles

Desde `core`, con Rust y el lockfile del repo:

```powershell
$env:CARGO_INCREMENTAL='0'
$env:CARGO_BUILD_JOBS='2'
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_PROFILE_TEST_DEBUG='0'
cargo test --locked -p archie-speech -p archie-voice -- --test-threads=1
cargo test --locked -p archie-core -p archie-cli -p archie-sentry -p archie-lan -p archie-state -p archied -p archie-workstation -- --test-threads=1
```

Los replays usan audio y ASR controlados en la frontera externa, el loop real,
SpeechSession y NemotronCognitive con su assembler. Cubren final precoz, cláusula
retenida, acknowledgements repetidos, corrección inmediata/tardía, error TTS y
shutdown. Tres pruebas unitarias cubren recibos viejos, espera de reproducción y
error que corta la cola sin esperar a main. No son benchmarks de un modelo.
La bitácora registra los resultados y limitaciones de la última ejecución.

## Verificación que requiere el runtime real

En este equipo no se encontró el SDK en `/opt/archie/nemo-speech` ni
`NEMO_SPEECH_PREFIX` configurado. No se instalaron modelos ni se ejecutó síntesis
real, micrófono, AEC, PulseAudio, captura LAN, impresora o imágenes ISO/WSL.
El build por defecto no compila los caminos CLI bajo `speech/live/rvc`.

`WSL-OPERATOR.md` declara que la imagen Operator no incluye voz ni Observatory y
que IRIS no fue probado dentro de esa imagen. Integrar sus scripts no incluye este
loop automáticamente en su rootfs. Claude debe empaquetarlo y Xavi probarlo después.
Su `wsl.conf` fija `automount.enabled=true` e `interop.enabled=false`, distinto del
runtime aislado descrito en FIRST-RUN. No se cambió la configuración de las distros
instaladas ni se verificó aquí el acceso a archivos del host de esa nueva imagen.
Los cuatro scripts Bash importados y el AST del instalador PowerShell pasaron
validación estática; no se ejecutó setup, importación, descarga ni operación de IRIS.

Después de que Claude prepare el entorno, compilar ahí con el SDK realmente
instalado (no simular su prefijo):

```bash
export NEMO_SPEECH_PREFIX=/opt/archie/nemo-speech
cd core
cargo check --locked -p archie-voice --features speech,live,rvc
cargo test --locked -p archie-voice --features speech,live,rvc
```

Instalar el runtime y voces Piper con los scripts del README en el entorno Python
que lance el bridge. Las versiones y modelos están fijados; esto no demuestra que
haya wheels para toda arquitectura. Verificar Linux x86_64/ARM64 por separado.
No ejecutar ambos consumidores del mismo micrófono a la vez.

Se verificaron las publicaciones fijadas [Piper 1.8.0](https://pypi.org/project/piper-tts/1.8.0/)
y [ONNX Runtime 1.30.0](https://pypi.org/project/onnxruntime/1.30.0/), y la API del
[source publicado de Piper](https://github.com/OHF-Voice/piper1-gpl/blob/639388b6317fc4731e91d53da42aea68fd4166ff/src/piper/voice.py).
Esto verifica existencia/API por lectura, no instalación, voz o rendimiento.
Los dos configs y las dos model cards remotas de las revisiones fijadas se leyeron
y coincidieron con los MD5 del installer; ambos sample rates son 22.050 Hz. Las
cards declaran las grabaciones inglesas de LibriVox en dominio público y españolas
con Unlicense. No se descargaron ni validaron aquí los pesos ONNX.

El stream del navegador espera el tiempo nominal de audio enviado: aún no tiene
un ACK de reproducción del cliente y no prueba que alguien lo haya oído. PulseAudio
drena el sink. Cancelar no preempta un kernel de síntesis bloqueado: se corta al
recibir el siguiente callback PCM; la escritura de audio también puede bloquear.
Sin AEC, el modo sin headphones cierra reconocimiento mientras Archie habla.
La detección de idioma y acknowledgements sigue siendo heurística; no es biometría.

## Benchmarks de latencia y convivencia

Medir en cada equipo, sin extrapolar desde FLOPS ni desde gaming. Registrar CPU/GPU,
driver, RAM/VRAM, sistema, commit, versiones SDK/backend, modelo y checksum,
cuantización, sample rate, contexto y parámetros. Separar cold start (carga de
modelos) de warm turn; un TTFT del LLM no representa speech-to-speech completo.

1. Preparar una lista pública de frases ES/EN: cortas/largas, cláusulas partidas,
   acknowledgements, correcciones e instrucciones. Usar audio autorizado temporal,
   no conversaciones privadas ni voces grabadas en Git. Borrar el material al terminar.
2. Ejecutar 5 calentamientos y al menos 30 repeticiones por escenario. Usar reloj
   monotónico para: fin acústico, primer partial/final ASR, liberación del floor,
   request del modelo, primer token útil, primer segmento, primer PCM y sink/ACK.
   Registrar duración de carga aparte. Si el backend no expone token timestamp,
   etiquetar primer segmento; no llamarlo TTFT.
3. Publicar p50/p95/p99 y número de muestras, RTF TTS/ASR, memoria máxima, CPU/GPU,
   errores, falsas interrupciones, turnos perdidos y retraso de cancelación desde
   voz detectada hasta último PCM reproducido. No sumar percentiles de etapas.
4. Repetir idle, carga normal del host y juegos reales escogidos por el usuario,
   registrando FPS y frame times del juego y latencia de Archie. Comparar Archie
   apagado/encendido en el mismo equipo/configuración. Medir audio del sink/loopback
   para latencia audible; el pacing del servidor no sustituye esa medición.
5. Comparar dentro de Cortex los backends disponibles: CUDA en guest/WSL, backend
   nativo autorizado por bridge, CPU y nodo separado. Nunca crear un segundo Cortex.
   Ejecutar la matriz solo en hardware disponible; marcar plataformas no probadas.

No hay estimación de milisegundos validada aquí para Spark, Halo, Mac mini, RTX 2080
o Titan. El plan permite obtenerla con escenarios y artefactos comparables.

## Continuidad de Sentry y dominios

Consultar `HANDOFF-2026-10-11-SENTRY-AUDITORIA.md`: integrar sus commits no convierte
stubs en conectores operativos ni instala workers. IPP standalone aún no implica
Intent/journal de Core; netwatch requiere revisar permisos, retención, payload en
captura, NDP IPv6, calibración/deduplicación y productores hacia WorldGraph/pilares.
Pairing/identidad, biometría, apps móviles y control completo del host siguen siendo
trabajo posterior. Conservar separación personal/laboral, Vault/REM y el bridge de
Santiago; no importar sin filtro contexto de trabajo a una conversación personal.
