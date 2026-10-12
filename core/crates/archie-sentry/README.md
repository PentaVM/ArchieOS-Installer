# Sentry bajo Core

Este crate implementa intenciones tipadas, resolución de recursos emparejados,
autorización con `archie-policy`, observación anterior y posterior, y recibos
durables. `Verified` exige una medición posterior que satisfaga la postcondición;
un ACK por sí solo no acredita el resultado.

## Operador Linux

`archie sentry-execute office-switch switch-on` prepara una intención en Core,
muestra el payload y exige escribir el digest completo dentro de 30 segundos.
La aprobación sólo vale en esa conexión, con el UID autorizado, el mismo proceso
y el mismo terminal en primer plano. No existe `--yes`. Las cuentas de agentes
deben estar separadas de los UIDs operadores; esto comprueba una sesión de terminal,
no biometría ni proximidad física. Windows no autentica este camino de actuación.

```toml
[sentry]
operator_uids = [1000] # Sustituir por la cuenta operadora del nodo Linux.

[sentry.home_assistant]
endpoint = "https://192.168.1.5:8123/"
token_file = "/etc/archie/secrets/home-assistant.token"

[[sentry.home_assistant.resources]]
resource_id = "office-switch"
entity = "switch.office"
```

El archivo de token debe ser regular, privado (`0600`), accesible al usuario de
archied, sin symlink y de máximo 4096 bytes. Se abre de nuevo por operación; la
rotación se comprueba con las cabeceras Authorization recibidas por una fixture.
Configurar una entidad no empareja el recurso: State debe contener su pairing
aprobado por el flujo confiable de Core. La policy por defecto continúa negando
actuation; declarar `safety_boundary_present` requiere una frontera real en el nodo.
No habilitarla sólo para conseguir que la operación pase.

`archie sentry-receipt <intent-id>` consulta el recibo durable. Una reserva sin
resultado se muestra `Indeterminate` y devuelve salida fallida; no se reenvía.
La preparación de climate comprueba °C antes del guard final y consume esa
preparación una sola vez, sin otra consulta previa al POST de actuación.

El CLI y el daemon se verifican como procesos separados contra HA de fixture,
con PTY, confirmación, journal PostgreSQL y exactamente una actuación observada.
El cierre tras dispatch conserva un fence consultable al volver a arrancar Core.
La evidencia actual y sus límites están en
[el informe del operador](../../../docs/superpowers/sentry-operator-verification-2026-10-03.md).

## Evidencia del motor anterior a esta entrega

En Windows pasaron 31 pruebas de contratos, motor y protocolos. En Linux pasan 31
y ocho adicionales con PostgreSQL 18.6, TimescaleDB 2.29.2 y pgvector 0.8.6:
reservas concurrentes, colisiones, reinicio de conexiones, recibos alterados,
migraciones y reproducción del motor sin nuevo I/O. Una prueba separada conservó
una reserva pendiente y un recibo tras detener el cluster aislado con
`pg_ctl -m immediate` y reiniciarlo. Esto no certifica una pérdida física de energía.

Core compone `SentryService` con su `StateStore` y su policy. Cinco pruebas con la
base aislada comprueban preparación vinculada al ID asignado, emparejamiento
revocado antes del envío, cancelación, sustitución de intención, reproducción sin
I/O y composición en un Runtime arrancado. La policy por defecto sigue negando
actuation; una autoridad de otro componente no puede omitirla.
La suite completa de Core y State pasó 202 pruebas en Linux. El lint de los tres
crates afectados pasó con `--no-deps -D warnings`; Fabric conserva un warning
anterior ajeno a este cambio. Se compiló con Rust 1.98.1; el MSRV 1.80 declarado
por el workspace aún no se ha comprobado para este conjunto de dependencias.

`PostgresJournal` usa la conexión y el bloqueo de escritores de Core. Conserva
intención, hash, reserva y resultado en una migración independiente. Una reserva
sin resultado se reproduce como `Indeterminate` y nunca se reenvía. Dos conexiones
no pueden reservar de nuevo el mismo ID. Un recibo terminado es inmutable; repetir
exactamente su escritura es idempotente. `MemoryJournal` es sólo una fixture.

Los adapters HTTP desactivan proxies y redirects, limitan cuerpos a 1 MiB y sólo
admiten direcciones IP locales configuradas. Home Assistant exige HTTPS fuera del
loopback. No hay opciones para omitir la verificación TLS.

Home Assistant consulta la entidad configurada y llama servicios concretos de
switch, media_player, light y climate. La observación conserva `last_reported` o
`last_updated` del proveedor; no sustituye su fecha por la hora de recepción.
La identidad de evidencia se conserva al volver a leer exactamente el mismo
reporte. El proveedor puede informar estados optimistas: estas pruebas validan el contrato
del adapter, no constituyen una certificación física de cada integración HA.
La temperatura sólo se admite cuando `GET /api/config` declara °C en
`unit_system.temperature`. Se comprueba al observar y nuevamente antes de llamar
`climate.set_temperature`; Fahrenheit y cambios de unidad se rechazan.

La consulta IPP (`Get-Printer-Attributes`) valida el request ID, tipos,
límites y truncaciones. IPP/IPPS sin puerto usan 631; se conservan los puertos
explícitos y los de endpoints HTTP/HTTPS. La Brother DCP-T720DW registrada en Windows fue localizada
por sus metadatos IPP, pero no respondió a TCP 631.

## Imprimir

`archie-print` es la herramienta del operador para imprimir en una impresora IPP de la
red local. Pregunta a la impresora qué es y qué formatos acepta, muestra qué se enviaría
(nombre, páginas, sha256) y sólo envía cuando la persona en el terminal escribe `print`.
Luego sigue el trabajo con `Get-Job-Attributes` hasta que la impresora informa cómo
terminó y entrega un recibo JSON. `completed` es lo que la impresora declara; no es una
medición de la bandeja de salida.

```sh
archie-print --printer ipp://192.168.1.232/ipp/print --text "Hola, soy Archie."
archie-print --printer ipp://192.168.1.232/ipp/print --expect-uuid urn:uuid:… informe.pdf
```

Un PDF, JPEG, PWG o URF se envía tal cual si la impresora declara ese formato. El texto
se dibuja en páginas A4 JPEG (`page.rs`, 200 ppp, tipografía fija 8×8 con las letras del
español), porque las impresoras domésticas rara vez aceptan `text/plain`. Un trabajo se
envía una sola vez: un fallo después del envío se informa y no se reintenta.

Verificado contra una impresora simulada (codec, envío único, seguimiento del trabajo).
**No verificado contra la Brother DCP-T720DW**: el 2026-10-11 no respondía en la red.
Imprimir todavía no pasa por `Intent`/journal/aprobación por digest de Core; ver
`docs/HANDOFF-2026-10-11-SENTRY-AUDITORIA.md`.

## Chromecast (CAST v2)

`adapters/chromecast.rs` habla CAST v2 sobre TLS (TCP 8009): tramas con prefijo de
longitud de 4 bytes y `CastMessage` protobuf (`prost`, sin `protoc`), CONNECT/CLOSE,
PING/PONG, GET_STATUS, LAUNCH, STOP y SET_VOLUME (nivel y mute). `apply` sólo envía y
espera la respuesta a su `requestId`; la postcondición se mide con un GET_STATUS nuevo
sobre el RECEIVER_STATUS del propio receptor (`ResourceState::Cast`). STOP lee la sesión
en `preflight`, antes del control final de Core. Play/pausa necesita el canal de media de
la aplicación y queda `Unsupported`. Un receptor que redondea el volumen produce
`Diverged`, no `Verified`.

**Qué se verifica en TLS:** el SHA-256 del certificado hoja (DER) debe coincidir con el
`certificate_sha256` que el operador fijó, y la firma del handshake se comprueba con la
clave de ese certificado (rustls + ring). Otro certificado falla con `Authentication`
antes de enviar un byte CAST. **Qué no:** cadena, nombre ni fechas (el pin es el ancla),
ni el desafío de autenticación de dispositivo de Cast (`tp.deviceauth`, CA raíz de
Google): un canal fijado prueba «el mismo equipo que se emparejó», no «un Chromecast
genuino». Si el equipo rota su certificado, todo falla cerrado hasta que el operador
apruebe la huella nueva. `archie sentry cast-probe <ip>` hace un handshake, imprime la
huella y no envía nada ni confía en nada.

```toml
[[sentry.chromecasts]]
resource_id = "living-cast"
name = "Living Room"
model = "Chromecast"
address = "192.168.1.30"
port = 8009
certificate_sha256 = "<64 hex en minúsculas, de archie sentry cast-probe>"
```

CLI (flujo revisado): `archie sentry launch|stop|volume|mute|unmute <resource> …`.
Probado contra un receptor CAST v2 en proceso sobre TLS loopback
(`tests/servers/cast.rs`), que exige CONNECT, hace PING antes de responder, difunde
cambios y cuantiza el volumen. **No probado contra un Chromecast físico.**

## Servidores de protocolo en proceso

`tests/servers/` implementa cada protocolo con estado propio, y
`tests/protocol_servers.rs` hace pasar cada adaptador por el motor completo (Intent →
policy → aprobación → observación → envío → observación) contra ellos:

| Servidor | Estado que guarda | Lo que exige |
|---|---|---|
| Roku ECP (`roku.rs`) | encendido, app activa, canales instalados, reproductor | con el panel apagado sólo PowerOn actúa; 404 para un canal no instalado |
| Home Assistant (`home_assistant.rs`) | entidades, atributos, `last_reported` | token Bearer; servicios por dominio; unidad de temperatura |
| MQTT 3.1.1 (`mqtt.rs`) | mensajes retenidos, suscripciones con `+`/`#` | CONNECT con credenciales; los dispositivos son clientes TCP aparte que guardan su estado y publican retenido |
| IPP (`ipp.rs`) | cola de trabajos (pendiente → procesando → completado) | `document-format-supported`; bytes que no son del formato declarado abortan el trabajo |
| ONVIF (`onvif.rs`) | perfiles, nonces vistos | digest WS-Security verificado y nonce no repetido |
| UPnP IGD (`igd.rs`) | tabla de mapeos | 718 si otro cliente tiene el puerto, 714/713 como un router |
| Matter (`matter.rs`) | nodos comisionados y sus clusters | un nodo no comisionado no responde; restricciones de setpoint |
| CAST v2 (`cast.rs`) | apps, sesiones, volumen, mute | TLS, CONNECT, PING/PONG, difusión de cambios, cuantización del volumen |

Matter no habla Matter en el cable: `chip-tool` es quien lo hace, así que el servidor es
un `CommandRunner` que interpreta el argv de chip-tool e imprime su formato de informe.
Los fixtures enlatados que quedan en `tests/adapters_*.rs` cubren respuestas malformadas u
hostiles que un servidor correcto no produce. **Todo esto es prueba de protocolo en
loopback; ningún dispositivo físico ha sido verificado.**

Lo que los servidores encontraron: un Roku en su pantalla de inicio responde
`<app>Roku</app>` sin `id` y el adaptador lo rechazaba; y el adaptador MQTT publicaba y
desconectaba, de modo que la observación posterior competía con el informe del
dispositivo. Ahora `apply` se suscribe al tema de estado antes de publicar y espera
(acotado a 3 s) el informe en vivo; la evidencia sigue siendo la observación del motor.

## Diagnóstico

El daemon actualizado ofrece `archie sentry-printer <resource-id>`. Core busca
únicamente bindings del operador en `archie.toml`; no admite URLs desde el cliente.
La consulta requiere que el `printer-uuid` informado coincida con el configurado.
Esa coincidencia de un UUID autodeclarado no constituye pairing autenticado.

```toml
[[sentry.printers]]
resource_id = "office-printer"
endpoint = "ipp://192.168.1.232:631/ipp/print"
expected_uuid = "urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b"
```

El CLI y un daemon de esta rama se arrancaron como procesos reales en una raíz
aislada y verificaron el recorrido hasta una fixture TCP IPP. La consulta a la
Brother desde ese mismo daemon devolvió `Transport`; continúa sin validación física.

```sh
cargo run -p archie-cli --bin archie -- sentry-printer office-printer
cargo run -p archie-sentry --bin archie-sentry-probe -- ipp ipp://<IP-local>:631/ipp/print
cargo test -p archie-sentry
```

Las ocho pruebas durables están ignoradas en el comando normal. Para ejecutarlas,
configurar localmente `ARCHIE_SENTRY_TEST_DATABASE_URL` con una base aislada de
PostgreSQL 18 y usar `cargo test -p archie-sentry --test postgres --test durable_engine -- --ignored`.
No imprimir la cadena de conexión ni guardarla en Git. Cada prueba crea y elimina
sólo su propio schema, cuyo nombre incluye un identificador aleatorio.

## Fronteras pendientes para producción

`CoreAuthority` es autoridad de composición dentro de un proceso confiable. No es
un comprobante remoto ni se deserializa desde JSON. Esta entrega conecta operador
local y rotación de token HA. Quedan el transporte Edge remoto autenticado, su
ciclo de claves, el enrolment verificable de dispositivos reales y la validación
de presencia física cuando el caso de uso la requiera.

La instalación del daemon actualizado, empaquetado ISO, ejecución
con dispositivos físicos, MQTT, Matter, routers, ONVIF, fusión IRIS y cámaras
requieren pruebas específicas. Este crate no declara esas integraciones listas.

Protocolos: [Home Assistant REST](https://developers.home-assistant.io/docs/api/rest/),
[RFC 8010](https://www.rfc-editor.org/rfc/rfc8010),
[RFC 8011](https://www.rfc-editor.org/rfc/rfc8011).
