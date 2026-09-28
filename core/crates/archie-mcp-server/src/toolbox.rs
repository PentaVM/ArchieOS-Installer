//! What a tool is, to this server: a name, a schema, the most sensitive grade its result
//! can carry, and whether calling it can change anything.

use archie_types::PrivacyGrade;
use serde_json::{Map, Value};
use std::future::Future;

/// The most sensitive data this server will hand to an IDE agent.
///
/// Two values, on purpose. `Public` is the default, because the model behind an IDE is a
/// third party whose retention nobody on the node has verified. `Internal` is for an
/// operator who has, and it is the ceiling: there is no clearance for sensitive,
/// restricted or secret data, so no flag can open one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clearance {
    Public,
    Internal,
}

impl Clearance {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "public" => Some(Self::Public),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }

    pub fn grade(self) -> PrivacyGrade {
        match self {
            Self::Public => PrivacyGrade::Public,
            Self::Internal => PrivacyGrade::Internal,
        }
    }

    pub fn covers(self, grade: PrivacyGrade) -> bool {
        grade <= self.grade()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, and changes nothing anywhere.
    ReadOnly,
    /// Can change something: an application's state, a file an application writes.
    Acts,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    /// The most sensitive grade the tool's result can carry.
    pub grade: PrivacyGrade,
    pub effect: Effect,
}

/// A tool's answer. `is_error` marks a failure the agent should see and can act on — a
/// daemon that is not running, a refused action — as opposed to a malformed call, which
/// the server answers as a protocol error before any tool runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
    /// Images beside the text, such as a render of a board, for a client that shows them
    /// to its model.
    pub images: Vec<ToolImage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolImage {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

impl ToolOutcome {
    pub fn json(value: &Value) -> Self {
        Self {
            text: serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into()),
            is_error: false,
            images: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            is_error: true,
            images: Vec::new(),
        }
    }
}

/// The tools a server offers.
pub trait Toolbox {
    fn tools(&self) -> Vec<ToolSpec>;

    /// Runs a tool the server has already checked exists and is within clearance.
    fn call(
        &mut self,
        name: &str,
        arguments: Map<String, Value>,
    ) -> impl Future<Output = ToolOutcome>;
}
