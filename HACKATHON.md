# Nebius x NVIDIA Global AI Hackathon

## Provenance

The migration baseline is `PentaVM/ArchieOS` commit `045e782` (11 September 2026),
recorded locally in the source repository as annotated tag
`pre-nvidia-migration-2026-09-11`. The source repository and its remote branches were
not rewritten or deleted.

Pre-existing ArchieOS work includes the Rust control plane, PostgreSQL/TimescaleDB and
pgvector state, capability-aware routing, policy and execution boundaries, native Qt
observatory, GeoFabric foundations, OSM building ingestion, C++ Analytical Cortex,
Sentry, capture agents, installer, and ArchISO packaging.

## Work in this repository

- Extracted the active runtime into an independent history and excluded the retired
  Python/PySide6 application, stale release records, caches, and generated output.
- Added explicit NVIDIA NIM and Nebius Token Factory provider kinds without coupling
  cognitive roles to either provider.
- Added an OpenAI-compatible inference transport with endpoint validation, bounded
  responses, environment-only credentials, timeout handling, and a second privacy gate
  immediately before network egress.
- Added a small CLI smoke path for model discovery, readiness, and chat requests.
- Added an optional NVIDIA package profile and staged provider manifests in the ISO.
- Replaced legacy CI with checks that represent the migrated Rust and C++ stack.

No benchmark numbers are claimed here. Test results and any demo measurements must come
from executed builds on identified hardware.
