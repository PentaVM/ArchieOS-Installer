//! Trusted in-process composition. No wire caller can assert human authentication.
use archie_policy::PolicyEngine;
use archie_sentry::adapters::ipp::IppClient;
use archie_sentry::{
    contracts::Intent,
    engine::{CoreAuthority, EdgeAdapter, Engine, EngineError, ExecutionGuard, ExecutionResult},
    registry::Registry,
};
use archie_state::{StateError, StateStore};

use std::sync::{Arc, Mutex};

pub use crate::sentry_config::{PrinterBinding, SentrySection};

struct StateGuard {
    state: Arc<StateStore>,
    policy: PolicyEngine,
}
impl ExecutionGuard for StateGuard {
    fn validate(&self, intent: &Intent, authority: &CoreAuthority) -> Result<(), EngineError> {
        authority.check_policy(intent, &self.policy)?;
        self.state
            .validate_prepared_device_operation(
                intent.operation_id,
                &intent.resource_id,
                intent.trace_id,
                &intent.digest()?,
            )
            .map_err(|_| EngineError::Authority)?;
        Ok(())
    }
}
pub struct SentryService {
    state: Arc<StateStore>,
    registry: Arc<Mutex<Registry>>,
    guard: Arc<StateGuard>,
}
struct CompositeGuard {
    state: Arc<StateGuard>,
    extra: Arc<dyn ExecutionGuard>,
}
impl ExecutionGuard for CompositeGuard {
    fn validate(&self, intent: &Intent, authority: &CoreAuthority) -> Result<(), EngineError> {
        self.state.validate(intent, authority)?;
        self.extra.validate(intent, authority)
    }
}
impl SentryService {
    /// Registry routes and policy are trusted Core configuration.
    pub fn new(
        state: Arc<StateStore>,
        registry: Arc<Mutex<Registry>>,
        policy: PolicyEngine,
    ) -> Self {
        let guard = Arc::new(StateGuard {
            state: state.clone(),
            policy,
        });
        Self {
            state,
            registry,
            guard,
        }
    }
    /// Returns the allocated intent that the operator must review. The input's
    /// positive operation ID is a placeholder and grants no permission.
    pub fn prepare(&self, mut intent: Intent) -> Result<Intent, EngineError> {
        intent.validate()?;
        let resource = intent.resource_id.clone();
        let trace = intent.trace_id;
        self.state
            .prepare_bound_device_operation(&resource, trace, |id| {
                intent.operation_id = id;
                intent
                    .digest()
                    .map_err(|_| StateError::Corrupt("invalid typed Sentry intent".into()))
            })
            .map_err(|_| EngineError::Authority)?;
        Ok(intent)
    }
    pub async fn execute(
        &self,
        intent: &Intent,
        authority: &CoreAuthority,
        adapter: &mut dyn EdgeAdapter,
    ) -> Result<ExecutionResult, EngineError> {
        let journal = self.state.sentry_journal()?;
        Engine::new(self.registry.clone(), journal)
            .with_guard(self.guard.clone())
            .execute(intent, authority, adapter)
            .await
    }
    pub(crate) async fn execute_with_guard(
        &self,
        intent: &Intent,
        authority: &CoreAuthority,
        adapter: &mut dyn EdgeAdapter,
        extra: Arc<dyn ExecutionGuard>,
    ) -> Result<ExecutionResult, EngineError> {
        let journal = self.state.sentry_journal()?;
        Engine::new(self.registry.clone(), journal)
            .with_guard(Arc::new(CompositeGuard {
                state: self.guard.clone(),
                extra,
            }))
            .execute(intent, authority, adapter)
            .await
    }
}
impl crate::Runtime {
    pub async fn sentry_home_status(&self, resource_id: &str) -> crate::control::ControlResponse {
        use crate::control::ControlResponse;
        if archie_sentry::contracts::validate_id(resource_id).is_err() {
            return ControlResponse::error("invalid Sentry home resource");
        }
        let Some(binding) = &self.config.sentry.home_assistant else {
            return ControlResponse::error("Sentry Home Assistant is not configured");
        };
        if !binding
            .resources
            .iter()
            .chain(&binding.telemetry_resources)
            .any(|r| r.resource_id == resource_id)
        {
            return ControlResponse::error("Sentry home resource is not configured");
        }
        let adapter = match binding.adapter() {
            Ok(adapter) => adapter,
            Err(_) => {
                return ControlResponse::error("Sentry home binding or credentials unavailable")
            }
        };
        match adapter.snapshot(resource_id).await {
            Ok(snapshot) => ControlResponse::SentryHomeStatus { snapshot },
            Err(error) => ControlResponse::error(format!("Sentry home query failed: {error}")),
        }
    }
    /// Queries one configured printer. Wire clients cannot supply a URL or expand
    /// the configured network scope. This never prepares or sends a print job.
    pub async fn sentry_printer(&self, resource_id: &str) -> crate::control::ControlResponse {
        use crate::control::ControlResponse;
        if archie_sentry::contracts::validate_id(resource_id).is_err() {
            return ControlResponse::error("invalid Sentry printer resource");
        }
        let Some(binding) = self
            .config
            .sentry
            .printers
            .iter()
            .find(|p| p.resource_id == resource_id)
        else {
            return ControlResponse::error("Sentry printer resource is not configured");
        };
        let printer = match IppClient::new(&binding.endpoint) {
            Ok(printer) => printer,
            Err(_) => return ControlResponse::error("invalid Sentry printer binding"),
        };
        match printer.get_printer_attributes(1).await {
            Ok(snapshot) if snapshot.uuid.as_deref() == Some(binding.expected_uuid.as_str()) => {
                ControlResponse::SentryPrinter {
                    resource_id: resource_id.into(),
                    snapshot,
                }
            }
            Ok(_) => ControlResponse::error("Sentry printer identity mismatch"),
            Err(error) => ControlResponse::error(format!("Sentry printer query failed: {error}")),
        }
    }

    /// Read-only driver planning for one configured printer: a live
    /// Get-Printer-Attributes probe followed by `drivers::analyze`. This never
    /// installs or executes anything; the planned command line is data.
    pub async fn sentry_printer_doctor(&self, resource_id: &str) -> crate::control::ControlResponse {
        use crate::control::ControlResponse;
        if archie_sentry::contracts::validate_id(resource_id).is_err() {
            return ControlResponse::error("invalid Sentry printer resource");
        }
        let Some(binding) = self
            .config
            .sentry
            .printers
            .iter()
            .find(|p| p.resource_id == resource_id)
        else {
            return ControlResponse::error("Sentry printer resource is not configured");
        };
        let printer = match IppClient::new(&binding.endpoint) {
            Ok(printer) => printer,
            Err(_) => return ControlResponse::error("invalid Sentry printer binding"),
        };
        let snapshot = match printer.get_printer_attributes(1).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return ControlResponse::error(format!("Sentry printer probe failed: {error}"))
            }
        };
        if snapshot.uuid.as_deref() != Some(binding.expected_uuid.as_str()) {
            return ControlResponse::error("Sentry printer identity mismatch");
        }
        let plan = archie_sentry::adapters::drivers::analyze(&snapshot);
        let (tier, detail) = match &plan.tier {
            archie_sentry::adapters::drivers::DriverTier::Driverless => (
                "driverless".to_owned(),
                format!(
                    "printer declares a driverless raster format; document-format-supported: {}",
                    snapshot.document_formats.join(", ")
                ),
            ),
            archie_sentry::adapters::drivers::DriverTier::NeedsPackages { packages, reason } => (
                "needs_packages".to_owned(),
                format!("{reason}; packages: {}", packages.join(" ")),
            ),
            archie_sentry::adapters::drivers::DriverTier::Unknown { considered } => (
                "unknown".to_owned(),
                format!("no known driver plan; attributes considered: {considered:?}"),
            ),
        };
        ControlResponse::SentryPrinterDoctor {
            resource_id: resource_id.into(),
            tier,
            detail,
            planned_command_line: plan.planned_command_line(),
        }
    }

    /// Uses the booted node's state and policy; default policy refuses actuation.
    pub fn sentry_service(&self, registry: Arc<Mutex<Registry>>) -> SentryService {
        SentryService::new(self.state.clone(), registry, self.policy().clone())
    }
}

/// Largest document the CLI stages for printing; mirrors the IPP adapter's
/// spool bound so staged content is never rejected at execution time.
const MAX_STAGE_BYTES: u64 = 16 * 1024 * 1024;

/// Sniff the document format from its bytes; anything unrecognized is refused
/// rather than guessed, because the printer only accepts declared formats.
pub fn sniff_print_format(document: &[u8]) -> Option<archie_sentry::contracts::PrintFormat> {
    use archie_sentry::contracts::PrintFormat;
    if document.starts_with(b"%PDF-") {
        Some(PrintFormat::Pdf)
    } else if document.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(PrintFormat::Jpeg)
    } else if document.starts_with(b"RaS2") {
        Some(PrintFormat::PwgRaster)
    } else if document.starts_with(b"UNIRAST\0") {
        Some(PrintFormat::UrfRaster)
    } else if !document.is_empty()
        && std::str::from_utf8(document).is_ok()
        && !document
            .iter()
            .any(|b| b.is_ascii_control() && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c))
    {
        Some(PrintFormat::PlainText)
    } else {
        None
    }
}

/// Hash a local document, verify its format is one the contract knows, and
/// stage it atomically (tmp file + rename) as `spool_dir/<sha256>` — the exact
/// name the IPP adapter reads back and re-verifies before sending. Staging is
/// idempotent: re-staging identical content is a no-op, and no partial file
/// ever appears under the content name.
pub fn stage_print_document(
    spool_dir: &std::path::Path,
    source: &std::path::Path,
) -> Result<(String, archie_sentry::contracts::PrintFormat), String> {
    use sha2::{Digest, Sha256};
    if !spool_dir.is_absolute() || !spool_dir.is_dir() {
        return Err("configured spool_dir is not an existing absolute directory".to_owned());
    }
    let metadata =
        std::fs::metadata(source).map_err(|_| "cannot inspect the document".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_STAGE_BYTES {
        return Err("document must be a non-empty regular file of at most 16 MiB".to_owned());
    }
    let document = std::fs::read(source).map_err(|_| "cannot read the document".to_owned())?;
    let sha256 = format!("{:x}", Sha256::digest(&document));
    let format = sniff_print_format(&document).ok_or_else(|| {
        "document format is not printable (PDF, JPEG, PWG/URF raster or plain text)".to_owned()
    })?;
    let target = spool_dir.join(&sha256);
    if target.is_file() {
        // Identical content was already staged; the name is the content hash.
        return Ok((sha256, format));
    }
    let tmp = spool_dir.join(format!(".staging-{}-{sha256}", std::process::id()));
    std::fs::write(&tmp, &document).map_err(|_| "cannot stage the document".to_owned())?;
    if std::fs::rename(&tmp, &target).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err("cannot move the staged document into the spool".to_owned());
    }
    Ok((sha256, format))
}
