# Sentry Local Operator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans.

**Goal:** Cerrar el camino operador Linux → Core → HA → recibo durable.
**Architecture:** Contexto del socket no serializable, challenge por conexión,
adapter configurado y guard de sesión compuesto con el guard State existente.
**Tech Stack:** Rust, Tokio, PostgreSQL, Home Assistant REST.
**Spec:** docs/superpowers/specs/2026-10-03-sentry-local-operator.md.

## Global Constraints
- Autenticación exclusivamente Linux Unix socket; sin constructor de identidad JSON.
- UIDs autorizados por defecto vacíos; máximo 32, únicos.
- Una intención pendiente por conexión; 30 segundos; un uso; sin reenvío.
- Tokens locales privados, regulares, no symlink, máximo 4096 bytes.
- State pairing y policy existentes se conservan, sin auto-enrolment.

## Review Focus
- Compromiso de un agente que comparte UID con operador: separación de cuentas es requisito explícito.
- EOF/pipelining durante observe/apply: cancelar y conservar fence durable.
- Cambio de pairing, proceso o terminal antes del envío: denegar.
- Cambios de token: recargar por operación y no filtrar contenidos en errores.
- Restart/confirmación repetida: ninguna aprobación se restaura o consume dos veces.

### Task 1: configuración y contexto local
**Files:** Core sentry_config.rs/operator.rs/config.rs; tests/sentry_operator.rs.
- [x] RED de UID permitido/denegado, binding HA inválido y archivo privado.
- [x] Implementar configuración y contexto del socket, GREEN.

### Task 2: aprobación y ejecución del daemon
**Files:** Core operator.rs/sentry.rs/control.rs, archied/main.rs, CLI/main.rs.
- [x] RED prepare/approve por socket real, digest alterado, pairing y policy.
- [x] Implementar sesión, lease auditada y ejecución con guard adicional.
- [x] Verificar EOF/repetición y secretos; CLI interactivo y daemon como procesos.

### Task 3: verificación y entrega
- [x] Suite Linux completa relevante, clippy y formato: 276 pruebas y caída separada.
- [x] Revisión fresca única de esta entrega; corregir hallazgos RED→GREEN.
- [x] Commit, informe de evidencia y próximos requisitos físicos concretos.
