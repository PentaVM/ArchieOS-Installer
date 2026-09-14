//! Which applications the agent may operate, and how.
//!
//! The same bargain as the module, provider, MCP and search registries: an application
//! that is not described here is one the agent will not touch, and an operator can read
//! the file and see every application their node can be made to click on.
//!
//! Defaults are the conservative end of every choice. Text is public-only until raised,
//! modified-key chords are off, and nothing an entry does not list is allowed.

use crate::action::{ActionKind, NamedKey, MAX_TEXT_CHARS};
use archie_types::PrivacyGrade;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What the agent may do with one application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPolicy {
    /// The application's `xdg_toplevel.app_id`, exactly as it reports it.
    pub app_id: String,
    pub display_name: String,
    /// Action kinds the agent may perform. Anything absent is refused.
    pub allow: BTreeSet<ActionKind>,
    /// The most sensitive text the agent may type into this application.
    #[serde(default = "public_grade")]
    pub max_text_grade: PrivacyGrade,
    #[serde(default = "default_max_text_chars")]
    pub max_text_chars: usize,
    /// Whether chords with modifiers (`Ctrl+S`) may be sent at all.
    #[serde(default)]
    pub allow_modified_keys: bool,
    /// Action kinds that are allowed but must be approved by a person each time.
    #[serde(default)]
    pub requires_approval: BTreeSet<ActionKind>,
    /// Keys that must be approved each time, whatever chord they end: `enter` in a
    /// terminal runs a command, `delete` in a file browser removes something.
    #[serde(default)]
    pub approval_keys: BTreeSet<NamedKey>,
}

fn public_grade() -> PrivacyGrade {
    PrivacyGrade::Public
}

fn default_max_text_chars() -> usize {
    256
}

impl AppPolicy {
    pub fn validate(&self) -> Result<(), PolicyError> {
        let id_ok = !self.app_id.is_empty()
            && self.app_id.len() <= 255
            && self
                .app_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
        if !id_ok {
            return Err(PolicyError::AppId(self.app_id.clone()));
        }
        if self.display_name.trim().is_empty() || self.display_name.chars().count() > 128 {
            return Err(PolicyError::DisplayName(self.app_id.clone()));
        }
        if self.max_text_grade == PrivacyGrade::Secret {
            // Credentials reach an application through the vault's broker, never through
            // keystrokes an agent composed.
            return Err(PolicyError::SecretText(self.app_id.clone()));
        }
        if self.max_text_chars == 0 || self.max_text_chars > MAX_TEXT_CHARS {
            return Err(PolicyError::TextBound(self.app_id.clone()));
        }
        if let Some(kind) = self
            .requires_approval
            .iter()
            .find(|kind| !self.allow.contains(kind))
        {
            // Approval for an action that is not allowed reads as a permission that exists.
            return Err(PolicyError::ApprovalForDisallowed {
                app_id: self.app_id.clone(),
                kind: *kind,
            });
        }
        if !self.approval_keys.is_empty() && !self.allow.contains(&ActionKind::Key) {
            return Err(PolicyError::ApprovalForDisallowed {
                app_id: self.app_id.clone(),
                kind: ActionKind::Key,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("app id {0:?} must be 1..=255 characters of [A-Za-z0-9._-]")]
    AppId(String),
    #[error("display name for {0} must be 1..=128 characters")]
    DisplayName(String),
    #[error("{0}: text may never be graded secret")]
    SecretText(String),
    #[error("{0}: max_text_chars must be 1..={MAX_TEXT_CHARS}")]
    TextBound(String),
    #[error("{app_id}: approval required for {kind:?}, which is not allowed")]
    ApprovalForDisallowed { app_id: String, kind: ActionKind },
    #[error("duplicate app id {0}")]
    Duplicate(String),
    #[error("stage policy is not valid: {0}")]
    Parse(String),
    #[error("cannot read stage policy: {0}")]
    Io(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    #[serde(default, rename = "app")]
    apps: Vec<AppPolicy>,
}

/// Every application the agent may operate.
#[derive(Debug, Default, Clone)]
pub struct StagePolicy {
    apps: BTreeMap<String, AppPolicy>,
}

impl StagePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(&mut self, app: AppPolicy) -> Result<(), PolicyError> {
        app.validate()?;
        if self.apps.contains_key(&app.app_id) {
            return Err(PolicyError::Duplicate(app.app_id));
        }
        self.apps.insert(app.app_id.clone(), app);
        Ok(())
    }

    pub fn app(&self, app_id: &str) -> Option<&AppPolicy> {
        self.apps.get(app_id)
    }

    pub fn apps(&self) -> impl Iterator<Item = &AppPolicy> {
        self.apps.values()
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// Parse a TOML document of `[[app]]` entries. Any invalid entry fails the whole
    /// document: a policy that silently skipped a typo would present it as a missing app.
    pub fn parse(toml_text: &str) -> Result<Self, PolicyError> {
        let file: PolicyFile =
            toml::from_str(toml_text).map_err(|e| PolicyError::Parse(e.message().to_string()))?;
        let mut policy = Self::new();
        for app in file.apps {
            policy.allow(app)?;
        }
        Ok(policy)
    }

    /// Load a policy file. A missing file is an empty policy: no applications is an honest
    /// state, and the agent then operates nothing.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PolicyError> {
        match std::fs::read_to_string(path.as_ref()) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(PolicyError::Io(format!("{}: {e}", path.as_ref().display()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> AppPolicy {
        AppPolicy {
            app_id: id.into(),
            display_name: "App".into(),
            allow: [ActionKind::Click].into_iter().collect(),
            max_text_grade: PrivacyGrade::Public,
            max_text_chars: 64,
            allow_modified_keys: false,
            requires_approval: BTreeSet::new(),
            approval_keys: BTreeSet::new(),
        }
    }

    #[test]
    fn a_minimal_entry_grants_only_what_it_names() {
        let policy = StagePolicy::parse(
            r#"
[[app]]
app_id = "org.example.App"
display_name = "Example"
allow = ["click"]
"#,
        )
        .unwrap();
        let entry = policy.app("org.example.App").unwrap();
        assert_eq!(entry.max_text_grade, PrivacyGrade::Public);
        assert!(!entry.allow_modified_keys);
        assert!(!entry.allow.contains(&ActionKind::Text));
        assert!(policy.app("org.example.Other").is_none());
    }

    #[test]
    fn invalid_entries_fail_the_whole_policy() {
        let mut secret = app("a");
        secret.max_text_grade = PrivacyGrade::Secret;
        assert_eq!(secret.validate(), Err(PolicyError::SecretText("a".into())));

        let mut approval = app("b");
        approval.requires_approval.insert(ActionKind::Text);
        assert!(matches!(
            approval.validate(),
            Err(PolicyError::ApprovalForDisallowed {
                kind: ActionKind::Text,
                ..
            })
        ));

        let mut keys = app("c");
        keys.approval_keys.insert(NamedKey::Enter);
        assert!(keys.validate().is_err());

        assert!(app("has space").validate().is_err());
        let mut long = app("d");
        long.max_text_chars = MAX_TEXT_CHARS + 1;
        assert_eq!(long.validate(), Err(PolicyError::TextBound("d".into())));

        let mut policy = StagePolicy::new();
        policy.allow(app("e")).unwrap();
        assert_eq!(
            policy.allow(app("e")),
            Err(PolicyError::Duplicate("e".into()))
        );

        assert!(matches!(
            StagePolicy::parse(
                "[[app]]\napp_id = \"x\"\ndisplay_name = \"X\"\nallow = [\"click\"]\nallw = true\n"
            ),
            Err(PolicyError::Parse(_))
        ));
    }

    #[test]
    fn a_missing_policy_file_operates_nothing() {
        let policy = StagePolicy::load("definitely/not/a/stage/policy.toml").unwrap();
        assert!(policy.is_empty());
    }
}
