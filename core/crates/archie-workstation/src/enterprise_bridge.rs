//! Synchronization bridge between local workstation data and external corporate APIs.
//!
//! Enforces strict ingress/egress filtering based on `PrivacyGrade`.

use archie_types::model::PrivacyGrade;
use archie_types::task::{KanbanCard, Task};
use archie_types::error::{Result, TypeError};
use archie_vault::reference::VaultRef;
use archie_vault::store::Vault;
use archie_types::time::Timestamp;

/// Represents an external corporate API endpoint or board.
#[derive(Debug, Clone)]
pub struct EnterpriseAdapter {
    pub name: String,
    pub api_key_ref: VaultRef,
    pub allowed_grades: Vec<PrivacyGrade>,
}

impl EnterpriseAdapter {
    pub fn new(name: impl Into<String>, api_key_ref: VaultRef) -> Self {
        Self {
            name: name.into(),
            api_key_ref,
            // By default, only public and internal data can be sent to external corporate boards.
            allowed_grades: vec![PrivacyGrade::Public, PrivacyGrade::Internal],
        }
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
    pub fn prepare_for_export(&self, task: &Task, grade: PrivacyGrade) -> Result<KanbanCard> {
        if !self.permits_egress(grade) {
            return Err(TypeError::InvalidTask(
                format!(
                    "Task {} cannot be exported due to strict privacy limits: grade {} not permitted",
                    task.task_id, grade.as_str()
                ).into()
            ));
        }

        // Return the safe DTO
        Ok(KanbanCard::from(task))
    }

    /// Synchronizes a batch of tasks to the external system.
    pub async fn sync_to_remote(&self, _cards: &[KanbanCard], vault: &mut Vault) -> Result<()> {
        let now_ms = Timestamp::now().as_millis();
        
        // Zero-Secret Leak: Fetch the credential ephemerally.
        // It returns a SealedCredential which we expose to memory and immediately discard.
        let sealed_cred = vault.broker_credential("enterprise_bridge", &self.api_key_ref, now_ms)
            .map_err(|e| TypeError::InvalidTask(format!("Vault access failed: {:?}", e).into()))?;
        
        // The exposed secret is a Zeroizing<String>. It will be cleared from memory as soon as it goes out of scope.
        let _secret = sealed_cred.expose();
        
        // In a real implementation, we would configure the reqwest::Client here using `_secret`
        // as a Bearer token or API key header. As soon as this function returns, the Zeroizing
        // struct wipes the credential from memory.
        Ok(())
    }
}
