//! Synchronization bridge between local workstation data and external corporate APIs.
//!
//! Enforces strict ingress/egress filtering based on `PrivacyGrade`.

use archie_types::model::PrivacyGrade;
use archie_types::task::{KanbanCard, Task};
use archie_types::error::{Result, TypeError};

/// Represents an external corporate API endpoint or board.
#[derive(Debug, Clone)]
pub struct EnterpriseAdapter {
    pub name: String,
    pub allowed_grades: Vec<PrivacyGrade>,
}

impl EnterpriseAdapter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
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
    pub async fn sync_to_remote(&self, _cards: &[KanbanCard]) -> Result<()> {
        // In a real implementation, this would make an asynchronous HTTP request using reqwest
        // to a corporate API (e.g., Trello, Jira, custom board).
        // Since we cannot leak secrets, the HTTP client must use vault-injected credentials
        // that are never passed down in plain text here.
        Ok(())
    }
}
