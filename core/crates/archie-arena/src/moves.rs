//! What the deciding model may answer, and how each answer becomes Stage actions.
//!
//! The model sees a capture of the window scaled to fit it, and answers in that image's
//! pixels. Each move is mapped back to the window's own pixels, clamped inside it, and cut
//! to what one Stage command may carry; nothing here decides whether it may happen — the
//! Stage policy and the gate do.

use archie_stage::{KeyChord, PointerButton, StageAction, StageText, StrokePoint};
use archie_stage::{MAX_STROKE_PATHS, MAX_STROKE_POINTS, MAX_STROKE_POINTS_IN_ALL};
use serde::Deserialize;

/// One decision: what the model thinks is happening, and what to do next.
#[derive(Debug, Clone, Deserialize)]
pub struct Decision {
    #[serde(default)]
    pub thought: String,
    #[serde(default)]
    pub actions: Vec<Move>,
}

/// One thing to do, in the capture's pixels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Move {
    Click { x: f32, y: f32 },
    DoubleClick { x: f32, y: f32 },
    Drag { from: [f32; 2], to: [f32; 2] },
    /// `b`, `ctrl+z`, `bracketright`: modifiers first, joined by `+`.
    Key { keys: String },
    Type { text: String },
    /// Lines drawn one after another, each a list of [x, y] points.
    Strokes { paths: Vec<Vec<[f32; 2]>> },
    Scroll { x: f32, y: f32, dy: i32 },
    Wait { ms: u64 },
    /// Something worth remembering for the next run, said by Archie itself.
    Note { text: String },
    Done,
}

/// How the capture the model saw maps onto the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Window pixels per capture pixel.
    pub scale: f32,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    fn point(&self, x: f32, y: f32) -> (i32, i32) {
        let px = (x * self.scale).round().clamp(0.0, self.width.saturating_sub(1) as f32) as i32;
        let py = (y * self.scale).round().clamp(0.0, self.height.saturating_sub(1) as f32) as i32;
        (px, py)
    }
}

/// The Stage actions a move becomes, or why it becomes none. A long list of lines becomes
/// as many commands as it takes to carry it.
pub fn stage_actions(step: &Move, frame: Frame) -> Result<Vec<StageAction>, String> {
    let left = PointerButton::Left;
    Ok(match step {
        Move::Click { x, y } => {
            let (x, y) = frame.point(*x, *y);
            vec![StageAction::Click { x, y, button: left }]
        }
        Move::DoubleClick { x, y } => {
            let (x, y) = frame.point(*x, *y);
            vec![StageAction::DoubleClick { x, y, button: left }]
        }
        Move::Drag { from, to } => {
            let (x, y) = frame.point(from[0], from[1]);
            let (to_x, to_y) = frame.point(to[0], to[1]);
            vec![StageAction::Drag { x, y, to_x, to_y, button: left }]
        }
        Move::Key { keys } => {
            let chord = KeyChord::parse(&keys.to_lowercase()).ok_or_else(|| format!("not a key: {keys}"))?;
            vec![StageAction::Key { chord }]
        }
        Move::Type { text } => vec![StageAction::Text { text: StageText::new(text.clone()) }],
        Move::Scroll { x, y, dy } => {
            let (x, y) = frame.point(*x, *y);
            vec![StageAction::Scroll { x, y, steps_x: 0, steps_y: (*dy).clamp(-20, 20) }]
        }
        Move::Strokes { paths } => {
            let mut commands = Vec::new();
            let mut batch: Vec<Vec<StrokePoint>> = Vec::new();
            let mut points_in_batch = 0;
            for path in paths {
                let mut line: Vec<StrokePoint> = path
                    .iter()
                    .map(|p| {
                        let (x, y) = frame.point(p[0], p[1]);
                        StrokePoint { x, y }
                    })
                    .collect();
                line.dedup();
                if line.len() == 1 {
                    // A dot: the pen goes down and up where it is.
                    line.push(line[0]);
                }
                for piece in line.chunks(MAX_STROKE_POINTS) {
                    if piece.len() < 2 {
                        continue;
                    }
                    if batch.len() >= MAX_STROKE_PATHS || points_in_batch + piece.len() > MAX_STROKE_POINTS_IN_ALL {
                        commands.push(StageAction::Strokes { paths: std::mem::take(&mut batch), button: left });
                        points_in_batch = 0;
                    }
                    points_in_batch += piece.len();
                    batch.push(piece.to_vec());
                }
            }
            if !batch.is_empty() {
                commands.push(StageAction::Strokes { paths: batch, button: left });
            }
            commands
        }
        Move::Wait { .. } | Move::Note { .. } | Move::Done => Vec::new(),
    })
}

/// The decision in a model's answer: the JSON object in it, wherever the model put it.
pub fn decision_of(text: &str) -> Result<Decision, String> {
    let start = text.find('{').ok_or("no JSON object in the answer")?;
    let end = text.rfind('}').ok_or("no JSON object in the answer")?;
    let value: serde_json::Value =
        serde_json::from_str(&text[start..=end]).map_err(|e| format!("the answer is not JSON: {e}"))?;
    // Moves that do not parse are dropped one by one, not the whole decision.
    let thought = value["thought"].as_str().unwrap_or("").to_string();
    let actions = value["actions"]
        .as_array()
        .map(|items| items.iter().filter_map(|item| serde_json::from_value::<Move>(item.clone()).ok()).collect())
        .unwrap_or_default();
    Ok(Decision { thought, actions })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Frame = Frame { scale: 2.0, width: 1000, height: 800 };

    #[test]
    fn a_click_lands_in_window_pixels_and_inside_the_window() {
        let actions = stage_actions(&Move::Click { x: 100.0, y: 50.0 }, FRAME).unwrap();
        assert_eq!(actions, vec![StageAction::Click { x: 200, y: 100, button: PointerButton::Left }]);
        let outside = stage_actions(&Move::Click { x: 900.0, y: -5.0 }, FRAME).unwrap();
        assert_eq!(outside, vec![StageAction::Click { x: 999, y: 0, button: PointerButton::Left }]);
    }

    #[test]
    fn many_lines_travel_in_as_few_commands_as_the_stage_allows() {
        let paths: Vec<Vec<[f32; 2]>> = (0..100).map(|i| vec![[i as f32, 0.0], [i as f32, 10.0]]).collect();
        let actions = stage_actions(&Move::Strokes { paths }, FRAME).unwrap();
        assert_eq!(actions.len(), 2, "64 lines, then 36");
        let dot = stage_actions(&Move::Strokes { paths: vec![vec![[5.0, 5.0]]] }, FRAME).unwrap();
        assert!(matches!(&dot[0], StageAction::Strokes { paths, .. } if paths[0].len() == 2));
    }

    #[test]
    fn a_decision_is_read_from_wherever_the_model_put_it_and_bad_moves_are_dropped() {
        let answer = "Sure! ```json\n{\"thought\": \"pick the brush\", \"actions\": [{\"do\": \"key\", \"keys\": \"b\"}, {\"do\": \"fly\"}, {\"do\": \"click\", \"x\": 3, \"y\": 4}]}\n```";
        let decision = decision_of(answer).unwrap();
        assert_eq!(decision.thought, "pick the brush");
        assert_eq!(decision.actions, vec![Move::Key { keys: "b".into() }, Move::Click { x: 3.0, y: 4.0 }]);
        assert!(stage_actions(&Move::Key { keys: "ctrl+z".into() }, FRAME).is_ok());
        assert!(stage_actions(&Move::Key { keys: "hyper+q".into() }, FRAME).is_err());
    }
}
