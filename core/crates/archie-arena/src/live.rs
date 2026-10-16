//! Acting live: a planner says what to do, IRIS turns every frame into controller input.
//!
//! The planner (a reasoning model, untimed) reads the task and the first screen and answers
//! with steps of *what* to do — "drive to the ball", "jump and roll left", "click the Export
//! button" — each for a number of seconds. While the clock runs, every captured frame goes to
//! IRIS (`POST /act`) with the step in force, and IRIS answers with the controller state it
//! chooses: keys and mouse buttons held, where the pointer acts, the kind of step. The runner
//! turns that state into Stage actions — only the changes: a key newly held is a `KeyDown`, a
//! key no longer held a `KeyUp`, and so for mouse buttons — and renews what stays held before
//! the Stage would let it go. Nothing here decides for IRIS; it only carries.

use archie_stage::{Key, PointerButton, StageAction};
use serde::Deserialize;
use std::collections::BTreeSet;

/// How often a key or button kept down is pressed again, below the Stage's 5 s hold limit.
pub const RENEW_HELD_MS: u128 = 3_000;

/// IRIS's answer for one frame (the fields the runner uses).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Control {
    #[serde(default)]
    pub step_type: String,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub buttons: Vec<String>,
    #[serde(default)]
    pub pointer: Vec<f32>,
    #[serde(default)]
    pub latency_ms: f32,
    /// Set when IRIS could not act on the frame. Every other field is then its default, which
    /// must not be read as "hold nothing": the first live run did, 126 times.
    #[serde(default)]
    pub error: Option<String>,
    /// What IRIS plans to hold on the frames after this one, `chunk_hz` apart: played out
    /// until the next decision lands. Empty from a head without chunks.
    #[serde(default)]
    pub chunk: Vec<Planned>,
    #[serde(default)]
    pub chunk_hz: f32,
}

/// One planned frame of a chunk.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Planned {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub buttons: Vec<String>,
}

/// A decision, stamped with when the frame it was made on was captured.
#[derive(Debug, Clone)]
pub struct Decision {
    pub number: u64,
    pub control: Control,
    pub frame_ms: u128,
}

impl Decision {
    /// What to hold at `now_ms`: the control for now until the first planned frame is due,
    /// then each planned frame in turn, and the last one past the end of the chunk. Time is
    /// counted from the frame, so a slow decision starts partway into its own chunk.
    pub fn at(&self, now_ms: u128) -> Control {
        let control = &self.control;
        if control.chunk.is_empty() || control.chunk_hz <= 0.0 {
            return control.clone();
        }
        let elapsed_s = now_ms.saturating_sub(self.frame_ms) as f32 / 1000.0;
        let index = (elapsed_s * control.chunk_hz).floor() as usize;
        if index == 0 {
            return control.clone();
        }
        let planned = &control.chunk[(index - 1).min(control.chunk.len() - 1)];
        Control {
            keys: planned.keys.clone(),
            buttons: planned.buttons.clone(),
            ..control.clone()
        }
    }
}

/// A plan step: what to do, for how long.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Step {
    #[serde(rename = "do")]
    pub what: String,
    #[serde(default = "default_seconds")]
    pub seconds: f32,
}

fn default_seconds() -> f32 {
    5.0
}

/// The planner's answer: the steps, and the skill IRIS acts in.
#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub skill: String,
    pub steps: Vec<Step>,
}

/// The step in force `elapsed` seconds into the run; the last one holds to the end.
pub fn step_at(steps: &[Step], elapsed: f32) -> Option<&Step> {
    let mut until = 0.0;
    for step in steps {
        until += step.seconds.max(0.1);
        if elapsed < until {
            return Some(step);
        }
    }
    steps.last()
}

/// IRIS's key names as the Stage names them. Modifiers cannot be held on the Stage (a held
/// modifier would change every key after it), so they map to nothing.
pub fn stage_key(name: &str) -> Option<Key> {
    let named = match name {
        "esc" => "escape",
        "pageup" => "page_up",
        "pagedown" => "page_down",
        "up" => "arrow_up",
        "down" => "arrow_down",
        "left" => "arrow_left",
        "right" => "arrow_right",
        "shift" | "ctrl" | "alt" => return None,
        other => other,
    };
    Key::parse(named)
}

pub fn stage_button(name: &str) -> Option<PointerButton> {
    match name {
        "left" => Some(PointerButton::Left),
        "right" => Some(PointerButton::Right),
        "middle" => Some(PointerButton::Middle),
        _ => None,
    }
}

/// What is held on the Stage right now, and when each was last pressed.
#[derive(Debug, Default)]
pub struct Held {
    pub keys: BTreeSet<String>,
    pub buttons: BTreeSet<String>,
    pub pressed_at_ms: std::collections::BTreeMap<String, u128>,
}

impl Held {
    /// The Stage actions that take the held state to what IRIS wants now: releases first,
    /// then presses, then renewals of anything held longer than `RENEW_HELD_MS`.
    pub fn actions_towards(
        &mut self,
        control: &Control,
        now_ms: u128,
    ) -> (Vec<StageAction>, Vec<String>) {
        let mut actions = Vec::new();
        let mut unsupported = Vec::new();
        let want_keys: BTreeSet<String> = control
            .keys
            .iter()
            .filter(|k| {
                let ok = stage_key(k).is_some();
                if !ok {
                    unsupported.push((*k).clone());
                }
                ok
            })
            .cloned()
            .collect();
        let want_buttons: BTreeSet<String> = control
            .buttons
            .iter()
            .filter(|b| stage_button(b).is_some())
            .cloned()
            .collect();
        for key in self.keys.difference(&want_keys) {
            actions.push(StageAction::KeyUp {
                key: stage_key(key).expect("held keys are stage keys"),
            });
            self.pressed_at_ms.remove(&format!("k:{key}"));
        }
        for button in self.buttons.difference(&want_buttons) {
            actions.push(StageAction::ButtonUp {
                button: stage_button(button).expect("held buttons are stage buttons"),
            });
            self.pressed_at_ms.remove(&format!("b:{button}"));
        }
        for key in &want_keys {
            let tag = format!("k:{key}");
            let due = self
                .pressed_at_ms
                .get(&tag)
                .is_none_or(|at| now_ms.saturating_sub(*at) >= RENEW_HELD_MS);
            if due {
                actions.push(StageAction::KeyDown {
                    key: stage_key(key).expect("filtered above"),
                });
                self.pressed_at_ms.insert(tag, now_ms);
            }
        }
        for button in &want_buttons {
            let tag = format!("b:{button}");
            let due = self
                .pressed_at_ms
                .get(&tag)
                .is_none_or(|at| now_ms.saturating_sub(*at) >= RENEW_HELD_MS);
            if due {
                actions.push(StageAction::ButtonDown {
                    button: stage_button(button).expect("filtered above"),
                });
                self.pressed_at_ms.insert(tag, now_ms);
            }
        }
        self.keys = want_keys;
        self.buttons = want_buttons;
        (actions, unsupported)
    }

    /// Everything still held, let go.
    pub fn release_all(&mut self) -> Vec<StageAction> {
        let mut actions: Vec<StageAction> = self
            .keys
            .iter()
            .filter_map(|k| stage_key(k))
            .map(|key| StageAction::KeyUp { key })
            .collect();
        actions.extend(
            self.buttons
                .iter()
                .filter_map(|b| stage_button(b))
                .map(|button| StageAction::ButtonUp { button }),
        );
        self.keys.clear();
        self.buttons.clear();
        self.pressed_at_ms.clear();
        actions
    }
}

/// Pixels close to car-soccer's "RESET!" popup colour (teal2, #7FF0E0) in an RGB frame.
pub fn teal_pixels(pixels: &[[u8; 3]]) -> usize {
    pixels
        .iter()
        .filter(|p| {
            (p[0] as i32 - 127).abs() <= 24
                && (p[1] as i32 - 240).abs() <= 20
                && (p[2] as i32 - 224).abs() <= 22
        })
        .count()
}

/// Counts flip resets: the popup appearing (teal rising above `threshold` pixels after being
/// below it). The popup lasts several frames; one appearance is one reset.
#[derive(Debug, Default)]
pub struct ResetCounter {
    pub resets: u32,
    showing: bool,
}

impl ResetCounter {
    pub fn observe(&mut self, teal: usize, threshold: usize) -> bool {
        let now = teal >= threshold;
        let rising = now && !self.showing;
        if rising {
            self.resets += 1;
        }
        self.showing = now;
        rising
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_in_force_follows_the_clock_and_the_last_holds() {
        let steps = vec![
            Step {
                what: "drive to the ball".into(),
                seconds: 3.0,
            },
            Step {
                what: "jump".into(),
                seconds: 1.0,
            },
        ];
        assert_eq!(step_at(&steps, 0.5).unwrap().what, "drive to the ball");
        assert_eq!(step_at(&steps, 3.2).unwrap().what, "jump");
        assert_eq!(step_at(&steps, 99.0).unwrap().what, "jump");
    }

    #[test]
    fn only_changes_reach_the_stage_and_held_input_is_renewed() {
        let mut held = Held::default();
        let drive = Control {
            keys: vec!["w".into(), "shift".into()],
            buttons: vec!["left".into()],
            ..Control::default()
        };
        let (first, unsupported) = held.actions_towards(&drive, 0);
        assert_eq!(first.len(), 2, "w down and boost down: {first:?}");
        assert_eq!(
            unsupported,
            vec!["shift".to_string()],
            "a modifier cannot be held"
        );
        assert!(
            held.actions_towards(&drive, 500).0.is_empty(),
            "nothing changed, nothing sent"
        );
        let renewed = held.actions_towards(&drive, 3_200).0;
        assert_eq!(
            renewed.len(),
            2,
            "held past 3 s: pressed again before the Stage lets go"
        );
        let turn = Control {
            keys: vec!["a".into()],
            ..Control::default()
        };
        let changed = held.actions_towards(&turn, 3_300).0;
        assert!(changed.contains(&StageAction::KeyUp {
            key: Key::parse("w").unwrap()
        }));
        assert!(changed.contains(&StageAction::ButtonUp {
            button: PointerButton::Left
        }));
        assert!(changed.contains(&StageAction::KeyDown {
            key: Key::parse("a").unwrap()
        }));
        assert_eq!(
            held.release_all(),
            vec![StageAction::KeyUp {
                key: Key::parse("a").unwrap()
            }]
        );
    }

    #[test]
    fn a_reset_popup_is_counted_once_while_it_shows() {
        let mut counter = ResetCounter::default();
        let frames = [0, 5, 400, 380, 350, 2, 0, 420, 10];
        let rising: Vec<bool> = frames.iter().map(|t| counter.observe(*t, 150)).collect();
        assert_eq!(counter.resets, 2);
        assert_eq!(rising.iter().filter(|r| **r).count(), 2);
        let teal = vec![[127, 240, 224]; 10];
        let grey = vec![[120, 120, 120]; 10];
        assert_eq!(teal_pixels(&[teal, grey].concat()), 10);
    }

    #[test]
    fn iris_key_names_map_to_stage_keys() {
        assert_eq!(stage_key("up"), Key::parse("arrow_up"));
        assert_eq!(stage_key("space"), Key::parse("space"));
        assert!(stage_key("q").is_some());
        assert!(stage_key("ctrl").is_none());
    }

    #[test]
    fn a_chunk_plays_out_from_the_frame_it_was_decided_on() {
        let plan = |keys: &[&str]| Planned {
            keys: keys.iter().map(|k| k.to_string()).collect(),
            buttons: vec![],
        };
        let control = Control {
            keys: vec!["w".into()],
            chunk: vec![plan(&["w", "space"]), plan(&["a"]), plan(&[])],
            chunk_hz: 16.0,
            ..Control::default()
        };
        let decision = Decision {
            number: 1,
            control,
            frame_ms: 1_000,
        };
        assert_eq!(decision.at(1_000).keys, vec!["w"], "now");
        assert_eq!(
            decision.at(1_070).keys,
            vec!["w", "space"],
            "the first planned frame, 62.5 ms on"
        );
        assert_eq!(decision.at(1_130).keys, vec!["a"]);
        assert!(decision.at(1_190).keys.is_empty());
        assert!(
            decision.at(9_000).keys.is_empty(),
            "past the chunk, its last frame holds"
        );
        let plain = Decision {
            number: 2,
            control: Control {
                keys: vec!["d".into()],
                ..Control::default()
            },
            frame_ms: 0,
        };
        assert_eq!(
            plain.at(5_000).keys,
            vec!["d"],
            "a head without chunks holds its one control"
        );
    }
}
