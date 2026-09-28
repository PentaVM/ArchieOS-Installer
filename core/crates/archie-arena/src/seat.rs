//! The Stage, as the arena and the practice use it: one connection, one operator, one
//! application. Every look and every action goes through the operator and the gate.

use crate::image::decode;
use archie_paint::bench::Sheet;
use archie_policy::Principal;
use archie_stage::{
    drain, ActOutcome, ActRequest, CommandKey, LookOutcome, Operator, StageAction, StageClient,
    StageWorld,
};
use archie_types::PrivacyGrade;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub type Client = StageClient<OwnedReadHalf, OwnedWriteHalf>;

/// Takes `flag` and the value after it out of `args`.
pub fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == flag)?;
    args.remove(at);
    (at < args.len()).then(|| args.remove(at))
}

/// A stage command key: 64 hex characters in a file.
pub fn load_key(path: &str) -> Result<CommandKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let hex = text.trim();
    if hex.len() != 64 {
        return Err(format!("{path}: a stage key is 64 hex characters"));
    }
    let mut bytes = [0_u8; 32];
    for (at, pair) in hex.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|_| "a key is hex")?;
        bytes[at] = u8::from_str_radix(pair, 16).map_err(|_| "a key is hex")?;
    }
    Ok(CommandKey::from_bytes(&bytes))
}

pub struct Seat {
    pub operator: Operator,
    pub client: Client,
    pub world: StageWorld,
    pub snapshots: PathBuf,
    pub app_id: String,
    /// Who the gate is told is acting.
    pub principal_id: String,
}

impl Seat {
    pub fn request(&self, action: StageAction, why: &str) -> Result<ActRequest, String> {
        let surface = self
            .world
            .surfaces_of(&self.app_id)
            .next()
            .ok_or_else(|| format!("{} has no window on the stage", self.app_id))?
            .surface;
        Ok(ActRequest {
            app_id: self.app_id.clone(),
            surface: Some(surface),
            action,
            text_grade: PrivacyGrade::Public,
            principal: Principal::Human {
                id: self.principal_id.clone(),
                authenticated: true,
            },
            task_id: None,
            user_present: true,
            approved: false,
            justification: why.chars().take(200).collect(),
        })
    }

    pub async fn look(&mut self) -> Result<Sheet, String> {
        drain(&mut self.client, &mut self.world)
            .await
            .map_err(|e| e.to_string())?;
        let request = self.request(StageAction::Snapshot { region: None }, "look at the window")?;
        match self
            .operator
            .look(
                &mut self.client,
                &mut self.world,
                &request,
                &self.snapshots,
                Duration::from_secs(10),
            )
            .await
            .map_err(|e| e.to_string())?
        {
            LookOutcome::Captured(snapshot) => decode(&snapshot.png),
            LookOutcome::NotCaptured(outcome) => Err(format!("no capture: {outcome:?}")),
        }
    }

    /// One frame for a live controller: the whole window drawn at most `max_width` wide by
    /// the compositor and handed over raw. The same gate and grant as `look`.
    pub async fn frame(&mut self, max_width: u32) -> Result<Sheet, String> {
        drain(&mut self.client, &mut self.world)
            .await
            .map_err(|e| e.to_string())?;
        let request = self.request(StageAction::Frame { max_width }, "watch the window")?;
        match self
            .operator
            .look(&mut self.client, &mut self.world, &request, &self.snapshots, Duration::from_secs(10))
            .await
            .map_err(|e| e.to_string())?
        {
            LookOutcome::Captured(snapshot) => decode(&snapshot.png),
            LookOutcome::NotCaptured(outcome) => Err(format!("no frame: {outcome:?}")),
        }
    }

    pub async fn act(
        &mut self,
        action: StageAction,
        why: &str,
        timeout: Duration,
    ) -> Result<ActOutcome, String> {
        // What the compositor said since the last action first: a dialog that opened moved the
        // window's geometry on, and a command prepared for the old one is refused as
        // surface_changed.
        drain(&mut self.client, &mut self.world)
            .await
            .map_err(|e| e.to_string())?;
        let request = self.request(action, why)?;
        self.operator
            .act(&mut self.client, &mut self.world, &request, timeout)
            .await
            .map_err(|e| e.to_string())
    }
}
