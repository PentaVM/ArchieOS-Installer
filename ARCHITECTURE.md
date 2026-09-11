# Architecture

ArchieOS separates persistent system authority from replaceable model workers.

| Boundary | Implementation | Responsibility |
|---|---|---|
| Control plane | `core/` (Rust) | identity, policy, leases, scheduling, routing, audit, state |
| Analytical cortex | `cortex/` (C++23) | bounded context, evidence graph, correction trajectories |
| Physical world | `sentry/`, `capture-agents/` | consent, provenance, observations, device input contracts |
| Geospatial display | `display/`, `archie-geofabric`, `archie-worldd` | isolated read-only world projection and OSM geometry |
| Data providers | `providers/`, `archie-registry` | declarative capability and provenance catalog |
| Model transport | `archie-model-runtime` | authenticated provider-neutral inference calls |
| Persistence | PostgreSQL, TimescaleDB, pgvector | durable state, temporal observations, vector retrieval |
| Distribution | `distro/` | services, installer, reviewed DB packages, ArchISO image |

Requests are classified for privacy and capability before the router selects a model.
The selected model descriptor states its locality and clearance. The transport repeats
that privacy check at the egress boundary, so a caller cannot bypass routing policy by
constructing a cloud request directly. Local NIM endpoints may use loopback HTTP;
remote endpoints require HTTPS.

NVIDIA is an adapter below this boundary. A NIM server can run on the same node, another
trusted fabric node, or a configured cloud endpoint. Nebius Token Factory is a cloud
provider through the same OpenAI-compatible contract. Neither provider owns ArchieOS
identity, memory, policy, or cognitive roles.

The native display remains a read-only client. Sentry observations retain provenance
and consent separately from visual presentation. Models cannot grant capabilities or
actuate hardware; those decisions stay in the policy and execution boundaries.

Detailed component contracts and threat boundaries remain in `docs/` and the numbered
RFCs.
