# Sentry bajo Core

Este crate implementa intenciones tipadas, resolución de recursos emparejados,
autorización con `archie-policy`, observación anterior y posterior, y recibos
durables. `Verified` exige una medición posterior que satisfaga la postcondición;
un ACK por sí solo no acredita el resultado.

## Estado comprobado

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

IPP implementa únicamente `Get-Printer-Attributes`, validando el request ID, tipos,
límites y truncaciones. IPP/IPPS sin puerto usan 631; se conservan los puertos
explícitos y los de endpoints HTTP/HTTPS. La Brother DCP-T720DW registrada en Windows fue localizada
por sus metadatos IPP, pero no respondió a TCP 631. No se enviaron trabajos.

## Diagnóstico

```sh
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
un comprobante remoto ni se deserializa desde JSON. Todavía se necesita el flujo
autenticado del operador, presencia verificable, transporte Edge autenticado y
rotación de credenciales. No deben exponerse los métodos de actuation como RPC
sin esas fronteras.

La instalación del daemon actualizado, empaquetado ISO, ejecución
con dispositivos físicos, MQTT, Matter, routers, ONVIF, fusión IRIS y cámaras
requieren pruebas específicas. Este crate no declara esas integraciones listas.

Protocolos: [Home Assistant REST](https://developers.home-assistant.io/docs/api/rest/),
[RFC 8010](https://www.rfc-editor.org/rfc/rfc8010),
[RFC 8011](https://www.rfc-editor.org/rfc/rfc8011).
