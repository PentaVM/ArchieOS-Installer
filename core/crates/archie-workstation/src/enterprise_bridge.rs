//! Synchronization bridge between local workstation data and external corporate APIs.
//!
//! Enforces strict ingress/egress filtering based on `PrivacyGrade`.

use archie_cerebra::{pillars::PillarContext, Observation, Trigger};
use archie_types::error::{Result, TypeError};
use archie_types::model::PrivacyGrade;
use archie_types::task::{KanbanCard, Task};
use archie_vault::reference::VaultRef;
use archie_vault::store::Vault;
use archie_vault::ContextDomain;

/// Represents an external corporate API endpoint or board.
#[derive(Debug, Clone)]
pub struct EnterpriseAdapter {
    pub name: String,
    pub api_key_ref: VaultRef,
    pub allowed_grades: Vec<PrivacyGrade>,
    /// Bound by trusted workstation configuration, never by an incoming webhook.
    pub context_domain: Option<ContextDomain>,
}

impl EnterpriseAdapter {
    pub fn new(name: impl Into<String>, api_key_ref: VaultRef) -> Self {
        Self {
            name: name.into(),
            api_key_ref,
            // By default, only public and internal data can be sent to external corporate boards.
            allowed_grades: vec![PrivacyGrade::Public, PrivacyGrade::Internal],
            context_domain: None,
        }
    }

    pub fn in_domain(mut self, domain: ContextDomain) -> Result<Self> {
        if !domain.is_work() {
            return Err(TypeError::InvalidTask(
                "enterprise adapter requires a work domain".into(),
            ));
        }
        self.context_domain = Some(domain);
        Ok(self)
    }

    /// Does not grant authority. Legacy unclassified tasks remain compatible;
    /// explicitly personal or malformed domains can never cross this boundary.
    pub fn context_visible(&self, task: &Task) -> bool {
        if task.labels.get("archie.context_kind").map(String::as_str) == Some("personal") {
            return false;
        }
        match task.labels.get("archie.context_domain") {
            Some(raw) => ContextDomain::try_from(raw.clone()).is_ok_and(|domain| {
                domain.is_work()
                    && self
                        .context_domain
                        .as_ref()
                        .is_none_or(|bound| bound == &domain)
            }),
            None => self.context_domain.is_none(),
        }
    }

    /// Prepare an already admitted task for the existing cognitive sequencer.
    /// This neither releases quarantine nor authorizes a tool or an outbound message.
    pub fn cognitive_observation(
        &self,
        task: &Task,
        context: &PillarContext,
        at_ms: i64,
    ) -> Result<Observation> {
        if !self.context_visible(task)
            || task.status.is_terminal()
            || task
                .labels
                .get("archie.quarantine")
                .is_some_and(|value| value != "false")
            || task.task_id.to_string() != context.task_id()
            || task.labels.get("archie.context_domain").map(String::as_str)
                != Some(context.domain().path())
        {
            return Err(TypeError::InvalidTask(
                "work context does not match an admitted task".into(),
            ));
        }
        Ok(context.observation(
            Trigger::SystemEvent {
                source: "enterprise_bridge".into(),
            },
            "prepare-work",
            at_ms,
        ))
    }

    /// Evaluates if a task's information is allowed to leave the fabric.
    ///
    /// The actual task implementation in archie-types doesn't explicitly store its PrivacyGrade,
    /// but in a real kernel this would be extracted from the task context, inputs, or labels.
    /// For this bridge, we assume the task or its content must explicitly declare if it's Restricted/Secret.
    pub fn permits_egress(&self, grade: PrivacyGrade) -> bool {
        self.allowed_grades.contains(&grade)
    }

    /// Prepares a task for export to the external API by converting it to a KanbanCard.
    /// Returns an error if the task contains data that is not allowed to be exported.
    pub fn prepare_for_export(&self, task: &Task) -> Result<KanbanCard> {
        if !self.context_visible(task)
            || (task.labels.get("archie.context_kind").map(String::as_str) == Some("work")
                && !task.labels.contains_key("archie.context_domain"))
        {
            return Err(TypeError::InvalidTask(
                "task context cannot enter this enterprise adapter".into(),
            ));
        }
        let is_quarantined = task
            .labels
            .get("archie.quarantine")
            .is_some_and(|value| value != "false");
        if is_quarantined {
            return Err(TypeError::InvalidTask(
                format!(
                    "Task {} cannot be exported: currently in quarantine",
                    task.task_id
                )
                .into(),
            ));
        }

        // Determine grade from labels
        let grade_str = task
            .labels
            .get("archie.privacy_grade")
            .or_else(|| task.labels.get("privacy_grade"))
            .map(|s| s.as_str())
            .unwrap_or("restricted"); // default to restricted if not specified

        let grade = match grade_str {
            "public" => PrivacyGrade::Public,
            "internal" => PrivacyGrade::Internal,
            "sensitive" => PrivacyGrade::Sensitive,
            "restricted" => PrivacyGrade::Restricted,
            "secret" => PrivacyGrade::Secret,
            _ => PrivacyGrade::Restricted,
        };

        if !self.permits_egress(grade) {
            return Err(TypeError::InvalidTask(
                format!(
                    "Task {} cannot be exported due to strict privacy limits: grade {:?} not permitted",
                    task.task_id, grade
                ).into()
            ));
        }

        // Return the safe DTO
        Ok(KanbanCard::from(task))
    }

    /// Synchronizes a batch of tasks to the external system.
    pub async fn sync_to_remote(&self, _cards: &[KanbanCard], _vault: &mut Vault) -> Result<()> {
        // Preserve the brokering interface for a real transport, but do not expose a
        // credential or claim delivery while there is no outbound implementation.
        Err(TypeError::InvalidTask(
            "enterprise transport is not configured".into(),
        ))
    }
}
