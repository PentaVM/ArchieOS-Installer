# IRIS perception fabric

IRIS converts structured application state and bounded ephemeral pixels into a revisioned
semantic scene. It does not decide or execute: Cerebra proposes, policy authorizes, and
Motor executes with a task-scoped lease.

The implemented v0.1 core lives in `archie-iris`: scene graph, fixed-point transforms,
damage/ROI extraction, source contracts, specialist interfaces, Visual Reflex and Cortex
boundaries, bounded working memory, ACP, semantic motor actions and shared-weight
multi-source scheduling. Production OS, browser, camera and model providers remain adapters
to be configured; their absence fails explicitly.

Foundry training is isolated in `PentaVM/archie-iris`. ArchieOS consumes only a bounded,
hash-verified deployment manifest and INT8 checkpoint. `archie iris verify-model` verifies
that boundary locally.
