# Feed de contexto IoT para IRIS

`archie-sentry::feed` convierte las tres fuentes de conocimiento de dispositivos de
ArchieOS en **un solo flujo tipado de eventos** para IRIS, el modelo de
clasificación y decisión del dueño (que vive en otro repositorio). Este módulo es
solo el lado productor: no contiene ningún consumidor.

| Fuente | `source` | Qué aporta |
|---|---|---|
| Inventario activo LAN (SSDP/mDNS/UPnP/Roku, `archie-lan`) | `lan_scan` | dispositivos vistos, con sus hechos declarados |
| Recibos y observaciones del motor Sentry | `sentry_engine` | ejecuciones verificadas, divergencias, estados medidos |
| Análisis pasivo de red (`archie sentry-network-analyze`) | `passive_network` | activos clasificados y cambios de inventario, no confiables |
| Watchdog TShark en el router del dueño | `router_watchdog` | metadatos mDNS por host, agregados por lectura |

## El evento

`ContextEvent`, esquema `schema_version: 1`, un objeto JSON por línea:

| Campo | Contenido |
|---|---|
| `schema_version` | siempre `1`; cualquier otro valor se rechaza |
| `event_id` | sha256 hex (64 caracteres, minúsculas) derivado del contenido: misma entrada, mismo id; una repetición deduplica |
| `kind` | `device_seen`, `device_gone`, `device_state_changed`, `network_observation`, `alert_observed`, `intent_executed` |
| `observed_at` | milisegundos Unix **de la fuente**; `null` si la fuente no tiene timestamp — es el marcador explícito de «sin timestamp de origen», nunca se sustituye por «ahora» |
| `origin` | `measurement` o `model_hypothesis`, copiado de la fuente; nunca se promueve una hipótesis a medición |
| `source` | una de las cuatro fuentes de la tabla anterior |
| `subject` | identificador del dispositivo o recurso, validado (≤128 caracteres, alfanumérico más `.` `-` `_` `:`) |
| `payload` | variante tipada y acotada, etiquetada por `type` (ver abajo) |
| `provenance` | ids de evidencia de la fuente (≤16 entradas de ≤128 caracteres); vacío cuando la fuente no tiene evidencia |

Todos los niveles usan `deny_unknown_fields`: un campo desconocido invalida el
documento. `parse_event` además reverifica que `event_id` coincida con el
contenido, así una línea alterada no pasa por válida.

### Payloads

- **`device_facts`**: mapa de hechos del inventario LAN (≤32 entradas, claves
  ≤64 y valores ≤256 caracteres; los valores más largos se truncan de forma
  determinista, nunca se inventan). Incluye `class`, `name`, `manufacturer`,
  `model`, `services`, `seen_by` y `state.*`. En `device_state_changed`,
  `changed_keys` nombra exactamente los hechos que cambiaron. En
  `device_gone`, `facts` conserva lo último conocido.
- **`resource_state`**: un `ResourceState` del contrato Sentry (`tv`,
  `printer`, `switch`, `light`, `climate`, `media`), con el `origin` de la
  observación que lo produjo.
- **`network_asset`**: `ipv4_address`, `candidates` (≤8; clase, `score` 0–100,
  `supporting_signals`) y `trusted`, que hoy es siempre `false`. Un informe
  pasivo que declare `trusted: true` se rechaza entero: la clasificación
  analítica no es identidad verificada.
- **`watchdog_traffic`**: `ipv4_address`, `packets`, `services` (≤16) y
  `unrecognized_names`. Los nombres se reducen al vocabulario finito que ya
  reconoce el importador pasivo (Roku, Cast, IPP/IPPS, RTSP, Home Assistant);
  cualquier otro nombre solo se cuenta. Nombres de personas o de dispositivos
  no entran en el feed.
- **`execution_outcome`**: `status`, `operation_id`, `intent_digest`,
  `replayed`, `dispatched`, `stages`, `before_state` y `after_state`. El
  digest permite a IRIS correlacionar con el intent revisado sin ver su
  contenido.

## Garantías

- **Separación medición/hipótesis.** `origin` se copia de la fuente. Un
  `ExecutionResult` cuyas observaciones sean hipótesis produce eventos con
  `model_hypothesis`. Las observaciones pasivas son medición de paquetes
  vistos, pero sus `candidates` siguen siendo heurística declarada.
- **Timestamps de la fuente.** El inventario LAN usa `scanned_unix`; el recibo
  usa `finished_at`; el watchdog usa el `frame_time_epoch` más reciente de la
  lectura. El análisis pasivo no tiene timestamps y sus eventos llevan
  `observed_at: null`. Ninguna vía rellena con el reloj local.
- **Deduplicación por `event_id`.** El id es sha256 del contenido completo
  (incluidos `kind`, `observed_at` y `provenance`), con mapas ordenados y
  serialización determinista. Reingerir la misma fuente produce los mismos
  ids; `dedupe_events` conserva la primera aparición de cada id.
- **Memoria acotada.** ≤4096 dispositivos o eventos por llamada, entradas
  JSON ≤4 MiB, ≤4 MiB por lectura del watchdog, ≤8192 líneas y ≤4096 hosts
  por poll, ≤256 archivos seguidos, eventos serializados ≤16 KiB, segmentos
  ≤32 MiB. Todo exceso es error o conteo, nunca pánico ni truncamiento
  silencioso de eventos.

## Deltas

`diff_snapshots(prev, next)` compara dos `Snapshot` (mapas ordenados de
sujeto → hechos) y emite `device_seen`, `device_gone` o
`device_state_changed` solo ante cambios reales. Un dispositivo cuyos hechos
solo se reordenaron no produce evento; un hecho modificado produce
exactamente un `device_state_changed` con sus `changed_keys`. El `observed_at`
del delta es el `taken_at` de la foto nueva, también en `device_gone`: la
ausencia se observa en el escaneo nuevo. «Ya no observado» no significa
«apagado» ni «intruso».

## Contrato de ingesta del watchdog del router

El dueño ejecuta un watchdog con TShark **en su router**, que captura de forma
continua y exporta archivos rotados. El contrato:

- **Directorio** configurado, con archivos `*.jsonl` (solo archivos
  regulares; otros nombres se ignoran).
- **Un objeto JSON por línea**, UTF-8, ≤4096 bytes por línea, con los mismos
  campos que acepta el importador pasivo, más el timestamp opcional:

  ```json
  {"ip_src":"192.168.1.2","udp_srcport":5353,"dns_ptr_names":["Office._ipp._tcp.local"],"frame_time_epoch":1790000000.5}
  ```

  `udp_srcport` debe ser `5353` (este contrato es solo metadatos mDNS IPv4,
  igual que el importador; no es un IDS ni un lector de PCAP).
  `frame_time_epoch` es opcional; sin él el evento lleva `observed_at: null`.
- **Rotación**: mover el archivo viejo y crear uno nuevo en la misma ruta.
  El lector sigue por archivo el inode, una huella de los primeros 64 bytes y
  el offset; un inode nuevo o una cabecera distinta reinicia el offset a 0.
  En Windows no hay inode estable: la huella de contenido y la regresión de
  longitud cubren la detección.
- **Cola parcial**: solo se leen líneas completas terminadas en `\n`. Una
  línea a medio escribir espera a completarse; no se pierde ni se duplica.
- **Líneas malformadas**: se cuentan y se saltan
  (`WatchdogReader::malformed_lines()`), nunca son fatales. Una línea mayor
  que el presupuesto de lectura (4 MiB) viola el contrato y detiene solo su
  propio archivo hasta que rote.
- **Idempotencia**: mismo archivo y mismo offset, cero eventos nuevos.
- **Agregación**: cada `poll()` emite un `network_observation` por host con
  los paquetes y servicios vistos en esa lectura, no un evento por paquete.

## Escritura JSONL

`FeedWriter` escribe un `ContextEvent` por línea en `segment-NNNNNN.jsonl`.
Cada evento serializado está acotado a 16 KiB (los mayores se rechazan, no se
recortan). Los segmentos rotan a 32 MiB: un segmento nuevo aparece de forma
atómica (archivo temporal + rename); dentro de un segmento el append es
simple. Al abrir un directorio existente se reanuda el último segmento si aún
tiene espacio. No hay autenticación en el archivo: el directorio es estado
local confiable.

## Contrato de consumo para IRIS

1. **Sondeo de segmentos**: leer los `segment-*.jsonl` en orden, línea a
   línea, con `parse_event` o validación equivalente (rechazar campos
   desconocidos y comprobar `event_id`). Guardar el propio offset por
   segmento para reanudar.
2. **Ids estables**: deduplicar por `event_id` ante relecturas o solapes;
   dos líneas con el mismo id son el mismo hecho.
3. **Deltas, no fotos**: para contexto largo, consumir los eventos de delta
   (`device_seen/gone/state_changed`) en lugar de inventarios completos; la
   foto base se reconstruye aplicando deltas en orden.
4. **Payloads tipados para clasificación**: cada variante es un vocabulario
   finito y acotado; no hay cadenas libres sin límite. `origin` y `trusted`
   separan lo medido de lo inferido y deben entrar como rasgos, no como
   hechos.
5. **Tiempo**: ordenar por `observed_at` solo dentro de una misma fuente; los
   eventos con `observed_at: null` no tienen orden temporal conocido y no
   deben tratarse como recientes.

## Lo que NO está hecho

- **No hay canal push en vivo.** IRIS sondea archivos; no hay socket, cola ni
  suscripción.
- **No hay autenticación del archivo del feed.** La protección es la del
  directorio local; nada firma las líneas más allá del id por contenido.
- **El consumidor de IRIS no está en este repositorio.** Aquí solo se produce
  el feed; la lectura, el modelo y sus decisiones viven en el otro checkout.
- **No hay cableado con el CLI ni configuración de daemon todavía.** El
  comando `archie context-feed` y la configuración del directorio del watchdog
  son una propuesta pendiente (ver la sección siguiente).
- **No se verificó contra un router real ni contra TShark instalado** en este
  host; el formato de línea se define por contrato y por paridad con el
  importador pasivo existente.

## Propuesta de cableado (pendiente, no implementada)

- `archie context-feed --watchdog-dir <dir> --out <dir> [--lan <inventario.json>]
  [--passive <reporte.json>] [--once]`: en modo servicio sondea el directorio
  del watchdog, aplica `diff_snapshots` sobre inventarios LAN sucesivos,
  convierte recibos del journal Sentry y anexa todo al writer con rotación.
- Configuración: `[sentry.context_feed]` con `watchdog_dir`, `out_dir` y
  `poll_ms`; límites fijos en el código, no configurables a valores mayores.
- Estado duradero del lector (offsets e inodes) en un archivo propio del
  directorio de salida, para que un reinicio no reinyecte historial.

## Validación reproducible

```sh
cd core
cargo test -p archie-sentry --test feed
```

19 pruebas: round-trip serde de los seis `kind`, rechazo de JSON malformado y
de campos desconocidos, verificación del `event_id`, reordenación de hechos
sin evento, un hecho cambiado = exactamente un `device_state_changed`,
aparición y desaparición, deduplicación por id, origen hipótesis conservado,
cola parcial del watchdog, rotación de inode, límite de bytes entre polls,
líneas malformadas contadas, reducción de nombres personales a un contador,
rotación de segmentos con líneas parseables y rechazo de eventos grandes.
Suite completa del crate: todas las demás pruebas pasan; el objetivo de
prueba `adapters_roku` (trabajo en curso de otro agente, fuera de este
módulo) no compilaba en el momento de esta ejecución.
