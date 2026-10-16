# Sentry: operador local y ejecución del daemon

El usuario pide terminar el sistema. Esta entrega conecta la implementación existente
al operador Linux y a Home Assistant sin inventar pairing ni presencia física.

Core obtiene UID/PID del socket Unix mediante `peer_cred`, nunca del request. Sólo
UIDs configurados explícitamente pueden preparar y confirmar. El CLI exige una
terminal interactiva, muestra la intención exacta y pide escribir su digest. Core
comprueba que el proceso autenticado conserva PID/start-time, sesión, grupo y el
mismo terminal de control, y sigue en su primer plano. Esta presencia significa
sesión interactiva del operador; no es biometría
ni prueba de proximidad física. Los agentes deben usar cuentas distintas de las
cuentas autorizadas de operadores. El administrador del host es raíz de confianza.

Preparación y confirmación usan la misma conexión. Sólo una intención pendiente,
30 segundos en reloj de pared y monotónico, digest completo y consumo incluso tras confirmación inválida. Cerrar
la conexión invalida la sesión. No se restauran aprobaciones tras reiniciar.
La reserva durable existente evita reenviar una operación después de un envío
ambiguo. No hay un parámetro `authenticated` o `present` en el protocolo.

Home Assistant usa bindings explícitos del operador y recursos ya emparejados en
StateStore; configurar una entidad no crea pairing. Los tokens se leen de archivos
regulares privados, acotados a 4096 bytes y rechazando symlinks. Se recargan por
operación para permitir rotación sin incluir secretos en el protocolo o journal.
El endpoint conserva los límites HTTPS/local y los timeouts del adapter existente.

Core conserva su policy: la ausencia de safety_boundary sigue negando actuation.
Una confirmación crea una lease de una operación y persiste la decisión antes de
observar/aplicar. El guard vuelve a comprobar sesión y State antes del envío.
EOF o tráfico adicional durante una operación cancela la ejecución y deja la
reserva pendiente si el envío ya pudo comenzar; nunca se reintenta automáticamente.
`sentry-receipt` permite consultar el recibo o fence después de reiniciar mediante
una nueva sesión autenticada. Una reserva sin recibo se muestra Indeterminate.
La preparación asíncrona del protocolo se completa antes del último guard:
en climate, la comprobación de °C nunca queda entre ese guard y el POST de actuación.

Windows mantiene diagnóstico read-only; TCP loopback no autentica operadores.
Se verifica con socket Unix, PostgreSQL y servidor HA de fixture, y después con
procesos CLI/daemon. Hardware HA, Edge remoto, Matter/MQTT/cámaras y la impresora
continúan requiriendo evidencia propia. No se declara producción física por fixtures.

Referencia API: [Tokio peer credentials](https://docs.rs/tokio/latest/tokio/net/unix/struct.UCred.html),
[lectura cancel-safe](https://docs.rs/tokio/latest/tokio/io/trait.AsyncBufReadExt.html).
