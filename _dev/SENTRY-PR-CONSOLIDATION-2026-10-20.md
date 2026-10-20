# Sentry — consolidación de PRs abiertos (2026-10-20)

Se unieron en una sola rama `sentry/integrated` los cuatro PRs abiertos de Sentry
que estaban encadenados, para revisarlos e integrarlos juntos contra `main`.

## PRs de origen

- #39 `codex/sentry-discovery` — base `main`. Monitor LAN autónomo, descubrimiento
  acotado por evidencia, sondas de router (CLI + controller + UPnP IGD).
- #43 `codex/sentry-host-wan-bridge` — base #39. Puente de captura en host Windows,
  capturas en vivo acotadas, contexto DNS configurado.
- #44 `codex/router-admin-library` — base #43. SDK de administración de router
  inscrito por el dueño, comando `DeviceBlock` (drop FORWARD por MAC) como Intent
  verificado, CLI `archie sentry-node block/unblock`.
- #42 `claude/sentry-ola3-cast-focos-servidores` — base `main`. Avisos hablados de
  Sentry durante el silencio conversacional.

## Verificación

- Todos los commits de los cuatro PRs quedan contenidos en `sentry/integrated`
  (`git cherry` sin faltantes salvo el conflicto resuelto, cuyo contenido queda en
  `c25e4b0`/`cc2f9c6`).
- Build nativo C++ de Sentry: 16/16 pruebas ctest pasan.
- Conflicto en `tests/adapters_router.rs` resuelto conservando ambos bloques de test.

## Corrección hecha durante la integración

`cc2f9c6`: el hand-off del admin de router no pasaba `protected_macs` a
`RouterConfig` (E0063). Se añadió el parámetro a `RouterAdmin::core_adapter` y el
test rechaza una lista inválida. Doc `SENTRY-ROUTER-ADMIN-SDK.md` actualizada.

## Notas de revisión

- SDK de admin de router: usuario alnum/_/-, comandos fijos, opciones ssh
  endurecidas primero, TOFU apagado por defecto, salida acotada. Correcto.
- `archie-lan` test `autonomy_cli` falla solo dentro del contenedor (la ruta
  192.0.2.2 no se considera privada ahí): dependiente del entorno, preexistente
  desde #39 (7e2e748). No es regresión de esta integración.
