# Trabajo autónomo — 2026-10-12 (Claude)

Nelson pidió trabajo autónomo durante el día: expandir el dominio (Sentry y lo que
alimenta a Cerebra) y llevarlo hacia producción. Una rutina despierta esta sesión
cada hora; cada despertar lee este archivo, toma la primera tarea no hecha y la
deja commiteada y empujada en `claude/sentry-ola3-cast-focos-servidores` (PR #36).

## Protocolo de cada despertar

1. `git fetch origin` y mirar el CI del PR #36. CI rojo es lo primero.
2. Tomar la primera tarea `[ ]` de la cola. Marcarla `[~]` mientras dura.
3. Antes de empujar: `cargo fmt --all -- --check`, `cargo clippy --workspace
   --all-targets --locked -- -D warnings` y las pruebas de los crates tocados.
4. Commit pequeño, push, anotar el resultado en `_dev/BITACORA.md` y aquí.
5. Límites que no se cruzan: GUI Quickshell/Vault y generación WSL (Claude/Xavi en
   otra línea), `archie-iris-runtime`, dominios personal/trabajo, `restricted` nunca a
   la nube. Nada se declara verificado en hardware si sólo pasó contra mocks.
6. Si el disco se llena: `cargo clean` en `core/` y repetir.

## Cola

- [ ] **CI verde.** `cargo fmt` del workspace (sólo estilo), `#include <array>` en
  `sentry/src/cli/main.cpp`, clippy `-D warnings` con toolchain 1.88, `cargo test
  --workspace`, `ctest` de cortex/sentry/visual/osm.
- [ ] **Sentry 11 — primera respuesta reversible.** Aviso por voz/Observatory de una
  alerta, sin bloquear nada. Bloqueo o cuarentena sólo como `Intent` aprobado.
- [ ] **Sentry 14 — IPv6.** `address-moved` con Neighbor Discovery (NA/NS) además de ARP.
- [ ] **Sentry 7 — baselines por dispositivo.** Mecanismo (volumen y destinos habituales,
  alerta por desviación) con prueba; la tasa de falsos positivos queda sin medir.
- [ ] **Sentry 13 — unidad systemd.** `systemd-analyze verify` y endurecimiento de
  `archie-netwatch.service`; documentar lo que sólo puede comprobarse en Arch real.
- [ ] **Endurecimiento general.** Revisar `unwrap`/`expect` en rutas de producción de
  `archie-sentry` y `archie-cerebra`, timeouts y límites de tamaño en E/S de red.

## Hecho
