use crate::{NodeId, Point, Rect, SceneGraph, SourceId};
use archie_policy::{Action, Lease, LeaseRejection, Operation, Principal, Resource};
use archie_types::{TaskId, Timestamp};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticAction {
    Activate {
        node: NodeId,
    },
    PointWithin {
        node: NodeId,
        x_permille: u16,
        y_permille: u16,
    },
    Observe {
        node: NodeId,
    },
}

impl SemanticAction {
    fn node(&self) -> &NodeId {
        match self {
            Self::Activate { node } | Self::PointWithin { node, .. } | Self::Observe { node } => {
                node
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedMotorAction {
    pub operation_id: u64,
    pub scene_revision: u64,
    pub source: SourceId,
    pub node: NodeId,
    pub node_bounds: Rect,
    pub target: Option<Point>,
    pub policy_action: Action,
    pub policy_resource: Resource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotorOutcome {
    pub observed_scene_revision: u64,
    pub outcome_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MotorError {
    #[error("semantic action is invalid")]
    InvalidAction,
    #[error("semantic target is missing or has no live coordinates")]
    MissingTarget,
    #[error("scene changed after the motor action was prepared")]
    StaleScene,
    #[error("policy refused the motor action: {0}")]
    Policy(LeaseRejection),
    #[error("motor executor failed")]
    Executor,
    #[error("world verification did not observe a newer scene revision")]
    VerificationFailed,
}

pub trait MotorExecutor {
    fn execute(&mut self, action: &PreparedMotorAction) -> Result<MotorOutcome, MotorError>;
}

pub struct MotorController;

impl MotorController {
    pub fn prepare(
        scene: &SceneGraph,
        action: &SemanticAction,
        operation_id: u64,
    ) -> Result<PreparedMotorAction, MotorError> {
        if operation_id == 0 {
            return Err(MotorError::InvalidAction);
        }
        let node = scene.node(action.node()).ok_or(MotorError::MissingTarget)?;
        let bounds = node.bbox.ok_or(MotorError::MissingTarget)?;
        let (target, policy_action) = match action {
            SemanticAction::Activate { .. } => (Some(bounds.center()), Action::Execute),
            SemanticAction::PointWithin {
                x_permille,
                y_permille,
                ..
            } => {
                if *x_permille > 1_000 || *y_permille > 1_000 {
                    return Err(MotorError::InvalidAction);
                }
                let x =
                    i64::from(bounds.x) + i64::from(bounds.width) * i64::from(*x_permille) / 1_000;
                let y =
                    i64::from(bounds.y) + i64::from(bounds.height) * i64::from(*y_permille) / 1_000;
                (
                    Some(Point {
                        x: i32::try_from(x).map_err(|_| MotorError::InvalidAction)?,
                        y: i32::try_from(y).map_err(|_| MotorError::InvalidAction)?,
                    }),
                    Action::Execute,
                )
            }
            SemanticAction::Observe { .. } => (None, Action::Read),
        };
        Ok(PreparedMotorAction {
            operation_id,
            scene_revision: scene.revision(),
            source: node.source.clone(),
            node: node.id.clone(),
            node_bounds: bounds,
            target,
            policy_action,
            policy_resource: Resource::Process {
                executable: format!("iris-source:{}", node.source.as_str()),
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute<E: MotorExecutor>(
        lease: &mut Lease,
        principal: Principal,
        task_id: Option<TaskId>,
        user_present: bool,
        prepared: &PreparedMotorAction,
        current_scene: &SceneGraph,
        now: Timestamp,
        executor: &mut E,
    ) -> Result<MotorOutcome, MotorError> {
        if current_scene.revision() != prepared.scene_revision
            || current_scene
                .node(&prepared.node)
                .and_then(|node| node.bbox)
                != Some(prepared.node_bounds)
        {
            return Err(MotorError::StaleScene);
        }
        let operation = Operation {
            principal,
            resource: prepared.policy_resource.clone(),
            action: prepared.policy_action,
            task_id,
            user_present,
        };
        lease
            .authorises_one_operation(&operation, prepared.operation_id, now)
            .map_err(MotorError::Policy)?;
        let outcome = executor.execute(prepared)?;
        if outcome.observed_scene_revision <= prepared.scene_revision {
            return Err(MotorError::VerificationFailed);
        }
        lease.consume();
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeKind, SceneEvent, SceneNode};
    use archie_policy::{CapabilityRequest, Constraints, PolicyEngine, Scope};
    use archie_types::{AgentId, TraceId};

    fn scene() -> SceneGraph {
        let mut scene = SceneGraph::default();
        scene
            .apply(
                0,
                &[SceneEvent::Upsert(SceneNode {
                    id: NodeId::new("button:1").unwrap(),
                    source: SourceId::new("browser:1").unwrap(),
                    kind: NodeKind::Button,
                    bbox: Some(Rect::new(100, 200, 80, 40).unwrap()),
                    parent: None,
                    track_id: None,
                    confidence_permille: 1_000,
                    uncertainty_permille: 0,
                    last_seen_ms: 1,
                    label: Some("Create".into()),
                    revision: 0,
                })],
            )
            .unwrap();
        scene
    }

    fn lease(action: Action, operation_id: u64) -> (Lease, Principal) {
        let principal = Principal::Agent {
            agent: AgentId::new(),
        };
        let record = PolicyEngine::default().decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: Resource::Process {
                executable: "iris-source:browser:1".into(),
            },
            scope: Scope::OneOperation {
                operation: operation_id,
            },
            constraints: Constraints::with_actions(vec![action]),
            justification: "activate a task-selected scene node".into(),
        });
        (record.lease.unwrap(), principal)
    }

    struct Executor {
        calls: usize,
        revision: u64,
    }

    impl MotorExecutor for Executor {
        fn execute(&mut self, _action: &PreparedMotorAction) -> Result<MotorOutcome, MotorError> {
            self.calls += 1;
            Ok(MotorOutcome {
                observed_scene_revision: self.revision,
                outcome_code: "scene_changed".into(),
            })
        }
    }

    #[test]
    fn semantic_node_resolves_to_live_center_coordinates() {
        let scene = scene();
        let prepared = MotorController::prepare(
            &scene,
            &SemanticAction::Activate {
                node: NodeId::new("button:1").unwrap(),
            },
            7,
        )
        .unwrap();
        assert_eq!(prepared.target, Some(Point { x: 140, y: 220 }));
    }

    #[test]
    fn no_lease_means_no_executor_call() {
        let scene = scene();
        let prepared = MotorController::prepare(
            &scene,
            &SemanticAction::Activate {
                node: NodeId::new("button:1").unwrap(),
            },
            7,
        )
        .unwrap();
        let (mut lease, principal) = lease(Action::Read, 7);
        let mut executor = Executor {
            calls: 0,
            revision: 2,
        };
        assert!(matches!(
            MotorController::execute(
                &mut lease,
                principal,
                None,
                true,
                &prepared,
                &scene,
                Timestamp::now(),
                &mut executor,
            ),
            Err(MotorError::Policy(LeaseRejection::ActionNotPermitted))
        ));
        assert_eq!(executor.calls, 0);
    }

    #[test]
    fn verified_execution_consumes_exactly_one_lease_use() {
        let scene = scene();
        let prepared = MotorController::prepare(
            &scene,
            &SemanticAction::Activate {
                node: NodeId::new("button:1").unwrap(),
            },
            7,
        )
        .unwrap();
        let (mut lease, principal) = lease(Action::Execute, 7);
        let mut executor = Executor {
            calls: 0,
            revision: 2,
        };
        let result = MotorController::execute(
            &mut lease,
            principal,
            None,
            true,
            &prepared,
            &scene,
            Timestamp::now(),
            &mut executor,
        )
        .unwrap();
        assert_eq!(result.observed_scene_revision, 2);
        assert_eq!(lease.uses, 1);
    }

    #[test]
    fn stale_scene_is_recompiled_before_execution() {
        let mut scene = scene();
        let prepared = MotorController::prepare(
            &scene,
            &SemanticAction::Activate {
                node: NodeId::new("button:1").unwrap(),
            },
            7,
        )
        .unwrap();
        scene
            .apply(
                1,
                &[crate::SceneEvent::TransformSubtree {
                    root: NodeId::new("button:1").unwrap(),
                    transform: crate::Affine2D::translation(10, 0),
                }],
            )
            .unwrap();
        let (mut lease, principal) = lease(Action::Execute, 7);
        let mut executor = Executor {
            calls: 0,
            revision: 3,
        };
        assert_eq!(
            MotorController::execute(
                &mut lease,
                principal,
                None,
                true,
                &prepared,
                &scene,
                Timestamp::now(),
                &mut executor,
            ),
            Err(MotorError::StaleScene)
        );
        assert_eq!(executor.calls, 0);
    }
}
