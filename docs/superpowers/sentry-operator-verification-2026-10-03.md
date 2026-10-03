# Sentry: operador local — 2026-10-03

La ruta Linux operador → Core → Home Assistant → journal durable está implementada
y verificada contra servicios de prueba. Esta entrega no certifica Sentry completo
para producción física. Rama: codex/sentry-closed-loop; base: 999f355.

## Implementado y comprobado

- UID/PID obtenidos del kernel del socket Unix; UIDs autorizados vacíos por defecto.
- Intención tipada preparada en Core; revisión del digest completo en CLI, misma
  conexión, plazo de 30 segundos con reloj monotónico y de pared, consumo único.
- Identidad de proceso, sesión, terminal y grupo en primer plano conservados hasta
  dispatch. Sustituir el terminal invalida la sesión.
- Pairing existente en State y policy de Core comprobados antes del envío.
  Configurar una entidad no concede pairing ni habilita la frontera física.
- Lease de una operación y decisión persistida en auditoría antes del I/O.
- Servicios HA de switch, media, light y climate. Climate comprueba °C antes del
  último guard, sin consulta intermedia antes del POST de actuación.
- Token privado, regular, acotado y sin symlink, recargado por operación. Una
  fixture verifica las cabeceras Authorization recibidas después de rotarlo.
- Cierre/pipelining cancelan la ejecución. Ante un posible envío queda un fence
  durable; sentry-receipt lo muestra Indeterminate y no reintenta.
- Windows conserva diagnóstico; loopback TCP no concede autoridad de operador.

La presencia comprobada es una sesión interactiva de la cuenta Linux, no biometría
ni proximidad física. Agentes y operadores deben usar UIDs diferentes. El
administrador del host sigue dentro de la raíz de confianza.

## Evidencia

**276 pruebas, cero fallos y cero ignoradas** en Core, State, CLI y Sentry. Una
prueba de caída del cluster se excluye de esa invocación y se ejecuta separadamente
en dos fases. Ocho pruebas del operador incluyen CLI y daemon como procesos
distintos, PTY, confirmación, servicio TCP HA y una sola actuación observada.
Cinco pruebas adicionales cubren credenciales/configuración, terminal y token.

Los dos hallazgos Important de la revisión independiente se reprodujeron primero
con pruebas fallidas y luego pasaron: cambio de PTY conservando PID/start-time,
y revocación durante la preparación climate. No quedaron hallazgos Critical o
Important abiertos de esa revisión. La rotación se probó también en HTTP.

El cluster aislado PostgreSQL 18 del puerto 55439 se detuvo con pg_ctl -m immediate,
se reinició y conservó reserva pendiente y recibo terminado. Esto comprueba
recuperación del cluster; no certifica pérdida física de energía.

Lint de Core, State, Sentry, CLI y archied: --all-targets --no-deps -D warnings.
Formato y git diff --check correctos. CLI y daemon compilaron con Rust 1.98.1.
Los 19 fuentes/manifiestos/lockfiles probados en Linux coinciden por SHA-256 con
el worktree Windows. El MSRV 1.80 todavía no se ha certificado.

La falta de espacio en C: interrumpió WSL y dejó artefactos de compilación corruptos.
Se recompilaron los afectados; el pase final terminó correctamente. Se liberaron
cachés npm/pip, entradas prunable de pnpm y archivos temporales antiguos del
instalador de Visual Studio, sin borrar fuentes, configuraciones o datos instalados.

## Reproducir

Construir primero archied y archie. Preparar PostgreSQL 18 aislado con las
extensiones TimescaleDB/pgvector usadas por State. Configurar localmente
ARCHIE_TEST_DATABASE_URL, ARCHIE_TEST_ADMIN_DATABASE_URL y
ARCHIE_SENTRY_TEST_DATABASE_URL; no incluir sus valores en Git o logs.

```sh
script -q -e -c 'cargo test -p archie-core -p archie-state -p archie-cli -p archie-sentry -j1 --offline -- --include-ignored --skip cluster_crash_preserves_dispatch_fence_and_completed_receipt' /dev/null
cargo clippy -p archie-core -p archie-state -p archie-sentry -p archie-cli -p archied --all-targets --no-deps -j1 --offline -- -D warnings
```

La prueba de caída exige el cluster de fixture 55439, sentry_fixture, y
ARCHIE_SENTRY_TEST_RECOVERY_PHASE=write/verify antes/después de su parada/reinicio.
Nunca dirigirla a la base instalada. Se utilizó lld para reducir memoria de enlace.

## Pendiente para producción completa

1. Despliegue al servicio instalado e ISO reproducible, con rollback, espacio
   suficiente y prueba bajo el usuario real del daemon. El servicio instalado
   archie-core sigue activo con su binario anterior; la entrega se probó aislada.
2. HA real, certificado/endpoint y token local; enrolment/pairing verificados y
   mediciones físicas de las postcondiciones. El pairing de fixtures no acredita hardware.
3. Transporte Edge remoto autenticado, permisos/expiración en el receptor,
   enrolment, revocación y rotación de claves. CoreAuthority local no lo sustituye.
4. MQTT, Matter, routers, ONVIF/cámaras y fusión IRIS: integración y evidencia propia.
5. Brother: continúa sin validación física. IPP consulta atributos; no implementa
   envío/cancelación de trabajos en esta entrega.

Mapa público: https://architecture-vercel-seven.vercel.app. Representa arquitectura
documentada; no es consola del daemon ni prueba de conexión con dispositivos.
Las fixtures nunca se presentan como hardware de producción.
