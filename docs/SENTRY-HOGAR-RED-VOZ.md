# Sentry: hogar, red y voz

## Dirección

Wanda se controla principalmente por voz. Pantallas y tarjetas son complementos para
revisar resultados y corregirlos. Archie Core sigue siendo la autoridad: el host,
Home Assistant, routers y satélites son proveedores y ejecutores, no otro Cortex.
IRIS queda fuera de estos cambios, conforme al trabajo ya realizado con Claude.

## Implementación de esta entrega

`archie sentry-home-status room --json` consulta exclusivamente una entidad configurada
por el operador. Usa GET `/api/states/<entity>`; no invoca `update_entity`, servicios,
emparejamiento ni operaciones físicas. Conserva el timestamp de Home Assistant:
consultar su caché no demuestra que el sensor acaba de medir ni que esté conectado.
`available` distingue `unknown` y `unavailable`; no certifica presencia o identidad.
La respuesta conserva estado, unidad y referencia reproducible; descarta atributos
arbitrarios, coordenadas y credenciales. Los sensores no entran en el registro actuable.

Ejemplo de configuración Linux, usando el lector privado de tokens existente:

```toml
[sentry.home_assistant]
endpoint = "https://192.168.1.10:8123"
token_file = "/etc/archie/secrets/home-assistant.token"
resources = []

[[sentry.home_assistant.telemetry_resources]]
resource_id = "room"
entity = "sensor.room_temperature"

[[sentry.home_assistant.telemetry_resources]]
resource_id = "door"
entity = "binary_sensor.front_door"
```

Límite conjunto: 64 recursos; IDs únicos. Se aceptan `sensor`, `binary_sensor`,
`device_tracker` y los cuatro dominios actuables existentes. La consulta no amplía
capabilities. El lector de tokens mantiene el requisito Linux de archivo regular
privado, sin symlinks; en Windows el daemon no puede abrir esos tokens actualmente.
HTTPS en LAN exige certificado confiable para la IP; no se desactiva TLS.

### Análisis pasivo de red

`archie sentry-network-analyze --scope 192.168.1.0/24 --input current.tsv
--baseline previous.tsv --json` analiza archivos locales sin daemon, sockets ni escaneos.
Reutiliza el plan, las reglas de clasificación y las diferencias existentes en Core.
No ejecuta Nmap. La ausencia significa «ya no observado», no «offline» ni «intruso».

Formato: tres campos TSV sin cabecera ni comillas, UTF-8, separados por tabulación:
IPv4 origen, puerto UDP origen y nombres PTR mDNS separados por coma. Máximo 1 MiB,
8192 líneas, 4096 bytes por línea y 4096 hosts. Todas las IP deben estar en el ámbito
RFC1918 declarado. Los nombres se reducen a señales finitas de Roku, Cast, IPP/IPPS,
RTSP y Home Assistant. No se guardan nombres de personas, dispositivos ni paquetes.
Cada resultado se marca como evidencia no confiable; no autoriza control ni pairing.

Exportación de una captura propia ya obtenida, desde Linux/WSL:

```sh
tshark -r home.pcapng -Y 'ip && udp.srcport == 5353 && dns.ptr.domain_name' \
  -T fields -E header=n -E separator=/t -E quote=n -E occurrence=a \
  -e ip.src -e udp.srcport -e dns.ptr.domain_name > current.tsv
archie sentry-network-analyze --scope 192.168.1.0/24 --input current.tsv --json
```

Este formato es solo mDNS IPv4; no es un IDS general ni un lector completo de PCAP.
Los metadatos de un paquete pueden ser falsificados y nunca sustituyen una identidad
firmada de Archie Fabric. No hay autenticación del productor de estos archivos.
TShark y dumpcap no se encontraron en el PATH del host durante esta auditoría;
el importador está implementado, pero la exportación desde una captura real todavía
necesita verificar la instalación de esas herramientas. La tabla de rutas local
identifica gateway `192.168.1.1` y enlace Ethernet en `192.168.1.0/24`; eso no
identifica el firmware ni concede acceso a la API de administración del router.

## Arquitectura siguiente

```text
Micrófono/satélite → VAD/wake word → STT → intención tipada → Core
                                                       ↙       ↘
                           Sentry: sensores/red/nodos       Wanda: interpretación
                                                       ↓
                              Policy + Scheduler + Forge/Cortex existente
                                                       ↓
                           ejecución → verificación → Vault → REM
                                                       ↓
                                      respuesta de voz + revisión opcional
```

Home Assistant puede alojar las integraciones de fabricantes y satélites de voz;
Archie decide tareas, modelos, memoria y política. No instalar un LLM paralelo en HA
para convertirlo en un segundo orquestador. STT, reconocimiento del hablante,
presencia y autorización son decisiones distintas. Una frase del dueño propone una
operación; la autorización procede de Core y de su política persistente.

### Red y blue team

- Primero telemetría autenticada del router: DHCP, vecinos, asociaciones Wi-Fi,
  contadores, cambios de configuración y reglas; adaptador según firmware.
- Después colector restringido `dumpcap`/TShark en interfaz configurada, puerto
  espejo/TAP o gateway. Un PC en Ethernet conmutado normalmente no ve unicast ajeno.
  TLS no se vuelve legible por habilitar promiscuous mode.
- Normalización: IPv4/IPv6, mDNS, SSDP, DHCP, DNS, flujos y timestamps; separar
  identidad, observación y atribución. Rotar capturas con cuotas y retención corta.
- Baseline por hogar/nodo y alertas explicables: dispositivo nuevo, DNS inesperado,
  cambios del router, servicio expuesto y aumento de conexiones. No diagnosticar
  malware a partir de una sola señal. Correlacionar con telemetría de endpoints.
- Contención mediante intents existentes: aislamiento/firewall reversibles,
  verificación y rollback. Un hallazgo no concede permisos nuevos automáticamente.

La captura, los adaptadores de router, la detección de amenazas y la contención aún
no están implementados por esta entrega. No se escaneó la red doméstica.

### Autonomía sobre los loops existentes

Core ya tiene `dispatch::run_pass`, reconciliación de ejecuciones inciertas,
contabilidad de slots, preemption y defer. Sentry ya tiene journal de operaciones y
fences contra repetir despachos ambiguos. Extender esos mecanismos:

1. Observar con presupuesto por sensor y conservar tiempo real de la fuente.
2. Planificar una tarea tipada con dominio personal/laboral, plazo y presupuesto.
3. Resolver permisos existentes, ejecutar y verificar efecto antes de declarar éxito.
4. Guardar evidencia mínima mediante Vault y usar REM en la implementación que ya
   existe en el checkout `ArchieOS-nvidia-unify`; no inventar otro almacén aquí.
5. Ceder recursos a interacción/juegos; obedecer detener/cancelar/tomar control por voz.
6. Limitar repeticiones, aplicar backoff, circuit breaker y reconciliar incertidumbre.

Un bucle infinito de prompts sin checkpoints, límites o verificación no constituye
autonomía fiable. Pendiente integrar eventos Sentry con el productor de contexto
personal/laboral previamente desarrollado y probar la cancelación de extremo a extremo.

### Nodos e iOS

PCs Linux/Windows/macOS usan agentes autenticados por Fabric y adaptadores locales
con capacidades declaradas. Android usa las APIs y el capture agent ya existente.
iOS requiere una app compañera con permisos del sistema, App Intents/Shortcuts y
APIs permitidas: no ofrece acceso arbitrario a otras apps ni un daemon universal
permanente. La actividad en background depende de modos y planificación de Apple.
Probar LAN y background en un iPhone físico; no certificarlo mediante simulador.

### Biometría, incluida voz

El código C++ ya tiene enrollment del dueño, lease, sensor, desafío temporal,
frescura, template binding y resultado con liveness. Es un control de autoridad,
no un matcher implementado. Reutilizarlo; no reemplazarlo por `voice_matches = true`.

Pendiente lector aislado real por modalidad: audio/micrófono y modelo de hablante,
sensor facial, huella mediante API del dispositivo, y otros sensores disponibles.
Los templates deben ir al Vault cifrado con dominio personal, borrado/revocación y
versiones. El worker debe vincular resultado al sensor y desafío actuales; sin prueba
de autenticidad del worker, recibir JSON no prueba identidad. La voz puede ayudar a
personalizar y atribuir contexto, pero puede reproducirse o sintetizarse: no aprobar
operaciones sensibles únicamente por parecido de voz. NIST 800-63B-4 excluye voz
como comparación biométrica para autenticación dentro de su marco.

No se implementó un lector biométrico universal ni se afirma reconocimiento real
de voz en esta entrega. Faltan adquisición, modelo, enrolamiento y pruebas con
audio genuino, replay, voz sintetizada, ruido, hablantes similares y falsos rechazos.

## Validación reproducible

```sh
cd core
cargo test -p archie-core -p archie-cli -p archie-sentry -- --test-threads=1
```

Los fixtures HTTP prueban GET exclusivo, timestamp histórico, respuestas idénticas,
sensor unavailable, identidad errónea, timestamp inválido, control characters y
rechazo de actuation sobre sensores. Pruebas de configuración separan sensores de
actuadores y conservan configuraciones anteriores. Las pruebas de red verifican
scope, reducción de nombres, deduplicación, límites y diferencias sin reclasificar
ausencia como fallo. Pruebas PostgreSQL y E2E ignoradas necesitan fixture aislado;
no equivalen a dispositivos reales.

Ejecución del 2026-10-11 en Windows, Rust 1.98.0: la suite de Core, CLI y Sentry
aprobó 177 pruebas, con 0 fallos y 27 omitidas.
La prueba adicional del CLI como proceso real también pasó: total 178 aprobadas,
0 fallos y 27 omitidas. Comprueba operación sin daemon, cambios respecto del
baseline, salida mínima JSON y rechazo del ámbito incorrecto.
Las pruebas de operator session
exclusivas de Linux tampoco se ejecutan en Windows. No se certificó el MSRV ni
el funcionamiento en router, Home Assistant, micrófono o iPhone reales.

Benchmarks pendientes en hardware: latencia HA p50/p95/p99 con 1/16/64 entidades;
fuente vs recepción y edad de caché; importación de 1 MiB y memoria pico; intervalo
de alerta tras un evento real; falsos positivos por 24 h; voz fin de frase→respuesta,
cancelación, uso CPU/RAM/GPU y frame time de juegos durante cargas concurrentes.
Registrar versiones, configuración, temperaturas, consumo y muestras; no estimar
compatibilidad universal desde un único juego o una cifra FP4.

## Fuentes primarias verificadas

- Home Assistant REST: https://developers.home-assistant.io/docs/api/rest/
- Assist pipelines: https://developers.home-assistant.io/docs/voice/pipelines/
- TShark: https://www.wireshark.org/docs/man-pages/tshark.html
- Ethernet conmutado: https://wiki.wireshark.org/CaptureSetup/Ethernet
- Apple LAN: https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
- Apple background: https://developer.apple.com/documentation/BackgroundTasks/choosing-background-strategies-for-your-app
- NIST biometría: https://pages.nist.gov/800-63-4/sp800-63b.html
