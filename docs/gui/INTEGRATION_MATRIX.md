# Matriz de integración: código → IPC → interfaz → prueba

Generada desde `CAPABILITY_REGISTRY.csv` (sólo capacidades con `ui_required` = `YES` o `INDIRECT`). Columna **Estado**: ninguna fila está `INTEGRATED` ni `TESTED`; `TODO` = sin diseño ni adaptador, `BLOCKED` = falta un contrato IPC o un backend (motivo en la columna IPC). `Prueba` es la referencia de prueba existente o `none yet`. Las filas `ADAPTER_TODO` indican que el adaptador tipado aún no existe.

| GUI | Capacidad | Código (ruta::símbolo) | IPC / contrato | Superficie > componente | Estado | Prueba |
|---|---|---|---|---|---|---|
| GUI-CORE-001 | CAP-CORE-001 | `core/crates/archie-core/src/control.rs::Status` | control.status (existing newline JSON) | CONTROL_CENTER > SystemSummaryCard | TODO | none yet |
| GUI-CORE-002 | CAP-CORE-002 | `core/crates/archie-core/src/control.rs::Topology` | control.topology | CONTROL_CENTER > FabricTopologyView | TODO | none yet |
| GUI-CORE-003 | CAP-CORE-003 | `core/crates/archie-core/src/control.rs::Modules` | control.modules | SETTINGS > ModuleStateList | TODO | none yet |
| GUI-CORE-004 | CAP-CORE-004 | `core/crates/archie-core/src/control.rs::Providers` | control.providers | WATCHER_FEED > ProviderHealthList | TODO | none yet |
| GUI-CORE-005 | CAP-CORE-005 | `core/crates/archie-core/src/control.rs::Peers` | control.peers | CONTROL_CENTER > PeerList | TODO | none yet |
| GUI-CORE-006 | CAP-CORE-006 | `core/crates/archie-core/src/control.rs::Devices` | control.devices | SENTRY_CENTER > PairedDeviceList | TODO | none yet |
| GUI-CORE-007 | CAP-CORE-007 | `core/crates/archie-core/src/control.rs::DeviceOperations` | control.device_operations | SENTRY_CENTER > OperationHistoryList | TODO | none yet |
| GUI-CORE-008 | CAP-CORE-008 | `core/crates/archie-core/src/control.rs::CortexTrajectories` | control.cortex_trajectories | DEVTOOLS > CortexInspector | TODO | none yet |
| GUI-CORE-009 | CAP-CORE-009 | `core/crates/archie-core/src/control.rs::Models` | control.models | SETTINGS > ModelRegistryList | TODO | none yet |
| GUI-CORE-010 | CAP-CORE-010 | `core/crates/archie-core/src/control.rs::Boot` | control.boot | CONTROL_CENTER > BootStageTimeline | TODO | none yet |
| GUI-CORE-011 | CAP-CORE-011 | `core/crates/archie-core/src/control.rs::Metrics` | control.metrics | CONTROL_CENTER > ResourceGauges | TODO | none yet |
| GUI-CORE-012 | CAP-CORE-012 | `core/crates/archie-core/src/control.rs::Audit` | control.audit | ACTIVITY_CENTER > AuditTimeline | TODO | none yet |
| GUI-CORE-013 | CAP-CORE-013 | `core/crates/archie-core/src/control.rs::Ping` | control.ping | CONTROL_CENTER > DaemonLivenessChip | TODO | none yet |
| GUI-CORE-014 | CAP-CORE-014 | `core/crates/archied/src/main.rs::main` | n/a | CONTROL_CENTER > DaemonLivenessChip | TODO | none yet |
| GUI-CORE-015 | CAP-CORE-015 | `core/crates/archie-cli/src/main.rs::Enrol` | ADAPTER_TODO (local file op; no control request) | SETTINGS > FabricEnrolWizard | TODO | none yet |
| GUI-CORE-016 | CAP-CORE-016 | `core/crates/archie-cli/src/main.rs::Doctor` | ADAPTER_TODO (call archie_workstation::health::doctor) | SETTINGS > DiagnosticsPanel | TODO | none yet |
| GUI-CORE-017 | CAP-CORE-017 | `core/crates/archie-cli/src/main.rs::Start` | ADAPTER_TODO (privileged helper; never shell-out from QML) | SETTINGS > SessionControls | TODO | none yet |
| GUI-CORE-018 | CAP-CORE-018 | `core/crates/archie-cli/src/main.rs::ProviderCommand` | ADAPTER_TODO | SETTINGS > ProviderManager | TODO | none yet |
| GUI-CORE-019 | CAP-CORE-019 | `core/crates/archie-cli/src/main.rs::HarnessCommand` | ADAPTER_TODO | SETTINGS > HarnessManager | TODO | none yet |
| GUI-CORE-020 | CAP-CORE-020 | `core/crates/archie-cli/src/main.rs::WorkspaceCommand` | ADAPTER_TODO | VAULT > WorkspaceLocations | TODO | none yet |
| GUI-CORE-021 | CAP-CORE-021 | `core/crates/archie-cli/src/main.rs::ToolsCommand` | ADAPTER_TODO | FORGE > ToolAdapterList | TODO | none yet |
| GUI-CORE-022 | CAP-CORE-022 | `core/crates/archie-cli/src/main.rs::SpeechCommand` | ADAPTER_TODO | SETTINGS > AudioDiagnostics | TODO | none yet |
| GUI-CORE-023 | CAP-CORE-023 | `core/crates/archie-cli/src/main.rs::IrisCommand` | ADAPTER_TODO | DEVTOOLS > IrisModelVerifier | TODO | none yet |
| GUI-POLICY-001 | CAP-POLICY-001 | `core/crates/archie-policy/src/lib.rs::PolicyEngine` | BLOCKED: no control request lets a human answer an escalation | ACTIVITY_CENTER > AuthorizationPrompt | BLOCKED | none yet |
| GUI-POLICY-002 | CAP-POLICY-002 | `core/crates/archie-policy/src/lib.rs::set_emergency_pause` | BLOCKED: no caller and no ControlRequest exposes it (grep) | CONTROL_CENTER > PanicPauseButton | BLOCKED | none yet |
| GUI-EXEC-001 | CAP-EXEC-002 | `core/crates/archie-executord/src/supervisor.rs::TaskRecord` | BLOCKED: no ControlRequest lists tasks (Status exposes only active_tasks) | ACTIVITY_CENTER > JobProgressList | BLOCKED | none yet |
| GUI-STAGE-001 | CAP-STAGE-001 | `core/crates/archie-stage/src/protocol.rs::StageEvent` | stage.events (existing JSON lines; commands HMAC-sealed) | WORKSPACE > StageWindowList | TODO | none yet |
| GUI-STAGE-002 | CAP-STAGE-002 | `core/crates/archie-stage/src/protocol.rs::AuthorityMode` | stage.events | ORB_CONTEXT > HandoverIndicator | TODO | none yet |
| GUI-STAGE-003 | CAP-STAGE-003 | `core/crates/archie-stage/src/action.rs::StageAction` | stage.events receipts | ACTIVITY_CENTER > AgentActionFeed | TODO | none yet |
| GUI-STAGE-004 | CAP-STAGE-004 | `core/crates/archie-stage/src/policy.rs::StagePolicy` | ADAPTER_TODO (read file via typed adapter) | SETTINGS > OperableAppsPolicyView | TODO | none yet |
| GUI-STAGE-005 | CAP-STAGE-007 | `core/crates/archie-stage/src/bin/archie-stage-cli.rs::main` | ADAPTER_TODO | DEVTOOLS > StageConsole | TODO | none yet |
| GUI-STAGE-006 | CAP-STAGE-009 | `core/crates/archie-arena/src/bin/archie-arena.rs::main` | ADAPTER_TODO | DEVTOOLS > ArenaRunsView | TODO | none yet |
| GUI-SIGHT-001 | CAP-SIGHT-001 | `core/crates/archie-sight/src/lib.rs::Sees` | ADAPTER_TODO | DEVTOOLS > IrisInspector | TODO | none yet |
| GUI-IRIS-001 | CAP-IRIS-001 | `core/crates/archie-iris/src/working_memory.rs::WorkingSceneMemory` | BLOCKED: no read API for IRIS state in the control protocol | DEVTOOLS > IrisInspector | BLOCKED | none yet |
| GUI-IRIS-002 | CAP-IRIS-003 | `core/crates/archie-iris-bridge/src/lib.rs::ProposedAction` | stage.events | ACTIVITY_CENTER > AgentActionFeed | TODO | none yet |
| GUI-IRIS-003 | CAP-IRIS-004 | `core/crates/archie-iris/src/foundry.rs::FoundryReflexModel` | CAP-CORE-023 | DEVTOOLS > IrisModelVerifier | TODO | none yet |
| GUI-VOICE-001 | CAP-VOICE-001 | `core/crates/archie-voice/src/bin/archie-voice.rs::main` | ADAPTER_TODO (process control; no daemon API) | ORB_CONTEXT > VoiceSessionControl | TODO | none yet |
| GUI-VOICE-002 | CAP-VOICE-002 | `core/crates/archie-voice/src/tools.rs::os_control` | stage.events | ACTIVITY_CENTER > AgentActionFeed | TODO | none yet |
| GUI-VOICE-003 | CAP-VOICE-003 | `core/crates/archie-voice/src/live/mod.rs::tools` | existing TCP 7668 DisplayControl | ORB_CONTEXT > VoiceToolReceipts | TODO | none yet |
| GUI-CEREBRA-001 | CAP-CEREBRA-001 | `core/crates/archie-cerebra/src/pass.rs::propose_stage_action` | ADAPTER_TODO | DEVTOOLS > CortexInspector | TODO | none yet |
| GUI-CEREBRA-002 | CAP-CEREBRA-002 | `core/crates/archie-cerebra/src/pillars.rs::PillarContext` | ADAPTER_TODO | ORB_CONTEXT > ContextDomainBadge | TODO | none yet |
| GUI-ROUTER-001 | CAP-ROUTER-001 | `core/crates/archie-router/src/lib.rs::RoutingOutcome` | ADAPTER_TODO | DEVTOOLS > RoutingExplainer | TODO | none yet |
| GUI-MODEL-001 | CAP-MODEL-001 | `core/crates/archie-model-runtime/src/lib.rs::ProviderEndpoint` | CAP-CORE-018 | SETTINGS > ProviderManager | TODO | none yet |
| GUI-SEALED-001 | CAP-SEALED-002 | `core/crates/archie-vault/src/store.rs::audit_log` | ADAPTER_TODO | SETTINGS > SealedAuditList | TODO | none yet |
| GUI-SEALED-002 | CAP-SEALED-003 | `core/crates/archie-vault/src/gateway.rs::ContextGateway` | ADAPTER_TODO | ACTIVITY_CENTER > DisclosureReceiptList | TODO | none yet |
| GUI-SEALED-003 | CAP-SEALED-004 | `core/crates/archie-vault/src/veil.rs::Veil` | ADAPTER_TODO | ACTIVITY_CENTER > DisclosureReceiptList | TODO | none yet |
| GUI-SEALED-004 | CAP-SEALED-005 | `core/crates/archie-vault/src/domain.rs::ContextDomain` | n/a | VAULT > DomainBadge | TODO | none yet |
| GUI-WORKSTATION-001 | CAP-WS-001 | `core/crates/archie-workstation/src/health.rs::doctor` | CAP-CORE-016 | SETTINGS > DiagnosticsPanel | TODO | none yet |
| GUI-WORKSTATION-002 | CAP-WS-004 | `core/crates/archie-workstation/src/harness.rs::UhpClient` | ADAPTER_TODO | FORGE > HarnessJobView | TODO | none yet |
| GUI-WORKSTATION-003 | CAP-WS-005 | `core/crates/archie-workstation/src/provider.rs::save_provider` | CAP-CORE-018 | SETTINGS > ProviderManager | TODO | none yet |
| GUI-WORKSTATION-004 | CAP-WS-006 | `core/crates/archie-workstation/src/enterprise_bridge.rs::EnterpriseAdapter` | BLOCKED: depends on board_api mounting | FORGE > BoardView | BLOCKED | none yet |
| GUI-WORKSTATION-005 | CAP-WS-007 | `core/crates/archie-workstation/src/board_api.rs::router` | BLOCKED: router referenced only by tests; no binary mounts it (grep) | FORGE > BoardView | BLOCKED | none yet |
| GUI-WORKSTATION-006 | CAP-WS-008 | `core/crates/archie-workstation/src/host.rs::workspace_path` | ADAPTER_TODO | VAULT > PathBoundaryChecker | TODO | none yet |
| GUI-SENTRY-001 | CAP-LAN-001 | `core/crates/archie-lan/src/inventory.rs::scan` | file FILE written by watch (existing) | SENTRY_CENTER > LanInventoryView | TODO | none yet |
| GUI-SENTRY-002 | CAP-SENTRY-001 | `sentry/include/sentry/camera/authority.hpp::CameraAuthority` | ADAPTER_TODO (C++ library; no IPC) | SENTRY_CENTER > PrivacyIndicators | TODO | none yet |
| GUI-SENTRY-003 | CAP-SENTRY-002 | `sentry/include/sentry/policy/consent.hpp::ConsentLease` | ADAPTER_TODO | SENTRY_CENTER > ConsentManager | TODO | none yet |
| GUI-SENTRY-004 | CAP-SENTRY-003 | `sentry/include/sentry/store/graph_store.hpp::GraphStore` | ADAPTER_TODO (CLI only) | SENTRY_CENTER > PersonGraphView | TODO | none yet |
| GUI-SENTRY-005 | CAP-SENTRY-004 | `sentry/include/sentry/identity/wanda.hpp::SocialHypothesis` | ADAPTER_TODO | SENTRY_CENTER > HypothesisReview | TODO | none yet |
| GUI-SENTRY-006 | CAP-SENTRY-005 | `sentry/include/sentry/capture/activity.hpp::CapturePolicy` | ADAPTER_TODO | SENTRY_CENTER > CaptureSourcesView | TODO | none yet |
| GUI-SENTRY-007 | CAP-SENTRY-006 | `sentry/include/sentry/identity/retention.hpp::Retention` | ADAPTER_TODO | SETTINGS > RetentionPolicyView | TODO | none yet |
| GUI-SENTRY-008 | CAP-SENTRY-007 | `sentry/include/sentry/policy/override.hpp::OwnerOverride` | ADAPTER_TODO | SENTRY_CENTER > OwnerOverrideDialog | TODO | none yet |
| GUI-SENTRY-009 | CAP-SENTRY-008 | `sentry/include/sentry/runtime/camera_worker.hpp::CameraWorker` | ADAPTER_TODO | SENTRY_CENTER > PrivacyIndicators | TODO | none yet |
| GUI-CORTEX-001 | CAP-CORTEX-001 | `cortex/include/archie/cortex/contracts.hpp::DeterministicContextCompiler` | cortex ipc.hpp kProtocolVersion=1 | DEVTOOLS > CortexInspector | TODO | none yet |
| GUI-DISPLAY-001 | CAP-DISPLAY-001 | `core/crates/archie-displayd/src/lib.rs::route` | existing HTTP loopback (no mutation) | CONTROL_CENTER > DisplaydClient | TODO | none yet |
| GUI-DISPLAY-002 | CAP-DISPLAY-002 | `core/crates/archie-displayd/src/kernel.rs::KernelSnapshot` | GET /api/kernel | CONTROL_CENTER > ResourceGauges | TODO | none yet |
| GUI-DISPLAY-003 | CAP-DISPLAY-003 | `display/native/src/AppCatalog.h::AppCatalog` | reuse C++ class or port logic (QML cannot exec) | DOCK > DockAppModel | TODO | none yet |
| GUI-DISPLAY-004 | CAP-DISPLAY-004 | `display/native/src/DisplayControl.cpp::DisplayControl` | existing TCP 7668 | ORB_CONTEXT > ObservatoryViewSwitch | TODO | none yet |
| GUI-WORLD-001 | CAP-WORLD-001 | `core/crates/archie-worldd/src/main.rs::main` | via displayd GET /api/world | WATCHER_FEED > WorldObservationFeed | TODO | none yet |
| GUI-WORLD-002 | CAP-WORLD-002 | `core/crates/archie-geofabric/src/lib.rs::FabricPlan` | via displayd | WATCHER_FEED > ProviderHealthList | TODO | none yet |
| GUI-REM-001 | CAP-REM-001 | `core/crates/archie-rem/src/main.rs::main` | ADAPTER_TODO | ACTIVITY_CENTER > RemStatusCard | TODO | none yet |
| GUI-MEMORY-001 | CAP-MEMORY-001 | `core/crates/archie-memory/src/admission.rs::AdmissionPolicy` | ADAPTER_TODO | DEVTOOLS > MemoryInspector | TODO | none yet |
| GUI-STATE-001 | CAP-STATE-001 | `core/crates/archie-state/src/snapshots.rs::capture` | ADAPTER_TODO | SETTINGS > BackupManager | TODO | none yet |
| GUI-STATE-002 | CAP-STATE-002 | `core/crates/archie-state/src/paths.rs::StatePaths` | n/a | VAULT > SystemLocations | TODO | none yet |
| GUI-FABRIC-001 | CAP-FABRIC-001 | `core/crates/archie-fabric/src/discovery.rs::Fabric` | CAP-CORE-005 | CONTROL_CENTER > PeerList | TODO | none yet |
| GUI-TOOLPACK-001 | CAP-TOOLPACK-001 | `core/crates/archie-toolpack/src/bin/archie-toolpack.rs::main` | ADAPTER_TODO | FORGE > ToolpackManager | TODO | none yet |
| GUI-TOOLPACK-002 | CAP-TOOLPACK-002 | `core/crates/archie-toolpack/src/unpack.rs::unpack_tar_gz` | reuse for Vault archive extract | VAULT > ArchiveHandler | TODO | none yet |
| GUI-BRIDGE-001 | CAP-BRIDGE-001 | `core/crates/archie-bridges/src/lib.rs::Bridge` | ADAPTER_TODO | FORGE > BridgeCatalog | TODO | none yet |
| GUI-BRIDGE-002 | CAP-BRIDGE-002 | `core/crates/archie-mcp-server/src/jobs.rs::specs` | MCP tool call or in-process equivalent | ACTIVITY_CENTER > JobProgressList | TODO | none yet |
| GUI-MCP-001 | CAP-MCP-001 | `core/crates/archie-mcp-server/src/archie.rs::ArchieToolbox` | MCP stdio | DEVTOOLS > McpToolCatalog | TODO | none yet |
| GUI-DESK-001 | CAP-DESK-001 | `core/crates/archie-desk/src/lib.rs::Desk` | files on disk (index.html + stages.json) | VAULT > DeliveriesLocation | TODO | none yet |
| GUI-DESK-002 | CAP-DESK-002 | `core/crates/archie-mcp-server/src/delivery.rs::specs` | MCP | ACTIVITY_CENTER > JobProgressList | TODO | none yet |
| GUI-SEARCH-001 | CAP-SEARCH-001 | `core/crates/archie-search/src/main.rs::main` | ADAPTER_TODO | LAUNCHER > UniversalSearchProvider | TODO | none yet |
| GUI-CUT-001 | CAP-CUT-001 | `core/crates/archie-cut/src/bin/archie-cut.rs::main` | ADAPTER_TODO | FORGE > RenderQueueView | TODO | none yet |
| GUI-OBS-001 | CAP-OBS-001 | `core/crates/archie-observability/src/audit.rs::AuditEvent` | CAP-CORE-012 | ACTIVITY_CENTER > AuditTimeline | TODO | none yet |
| GUI-FORGE-001 | CAP-FORGE-001 | `modules/core/forge.toml::core.forge` | BLOCKED: worker.kind="unimplemented" | FORGE > ForgeWorkspace | BLOCKED | none yet |
| GUI-WATCHER-001 | CAP-WATCHER-001 | `display/visual/include/archie/visual/scene.hpp::Watcher` | via displayd | WATCHER_FEED > WorldObservationFeed | TODO | none yet |
| GUI-MCP-002 | CAP-MCP-002 | `core/crates/archie-mcp/src/gate.rs::McpGate` | ADAPTER_TODO | ACTIVITY_CENTER > AgentActionFeed | TODO | none yet |
| GUI-REGISTRY-001 | CAP-REGISTRY-001 | `core/crates/archie-registry/src/providers.rs::ProviderManifest` | CAP-CORE-003 / CAP-CORE-004 / CAP-CORE-009 | SETTINGS > ProviderHealthList | TODO | none yet |

## Vault (gestor de archivos)

`GUI-VAULT-001..024`: sin código de origen; todos `BLOCKED` por ausencia de `archie-vaultd` (ADR-GUI-003). Contrato de autoridad previsto: `Resource::Filesystem` + `Action::{Read,Write,Delete}` + `Scope::OneOperation` (`core/crates/archie-policy/src/lib.rs`).

