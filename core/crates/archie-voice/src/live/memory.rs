//! What IRIS has seen over time: the things in front of the camera now, and the ones that
//! were there and are not any more.
//!
//! One frame says what is there; it does not say what left. Told once "a phone in your
//! hand", a model repeats it after the person has left the room, because nothing it is
//! shown says otherwise. The memory keeps each thing as a track — its category, where it
//! was, when it was first and last seen — so the context can say plainly what is seen now,
//! what was seen before and how long ago, and that nobody is in front of the camera.
//!
//! A thing is followed from frame to frame by its instance embedding, the head IRIS
//! distilled from DINOv2 so that the same object lands close in embedding space, and by
//! where its box is when there is no embedding. People are followed by place only: no
//! embedding of a person is kept, because telling one person from another is
//! identification, which needs their consent.
//!
//! A thing a hand touches for a few frames is held, and the memory keeps when it last was,
//! so "what did I have in my hand a moment ago?" is answered from it after the thing was
//! put down or left the frame. Whose hand is not known: hands are not identified either.

use crate::live::eyes::{is_person, Scene};
use serde::Serialize;
use std::time::{Duration, Instant};

/// A thing that has not been seen for this long is gone from view.
pub const GONE_AFTER: Duration = Duration::from_secs(3);
/// And one not seen for this long is forgotten.
const FORGET_AFTER: Duration = Duration::from_secs(30 * 60);
const CAPACITY: usize = 200;
/// Frames a hand must touch a thing before it counts as held: one is a hand passing by.
const HELD_AFTER_FRAMES: u32 = 2;
/// How much of the smaller of a hand's and a thing's boxes must overlap to count as touching.
const TOUCH: f32 = 0.15;
/// A thing held within this long is told as held.
const HELD_TOLD_FOR: Duration = Duration::from_secs(10 * 60);

/// One thing followed across frames.
#[derive(Debug, Clone)]
pub struct Thing {
    pub id: u32,
    pub phrase: String,
    pub bbox: [f32; 4],
    embedding: Option<Vec<f32>>,
    pub first_seen: Instant,
    pub last_seen: Instant,
    /// Frames it was seen in. A thing seen in a single frame may be a mistake of IRIS's,
    /// and is not remembered as having been there.
    pub sightings: u32,
    pub best_score: f32,
    /// Consecutive frames a hand touched it; once that reaches [`HELD_AFTER_FRAMES`], when
    /// it was first and last held.
    touched_frames: u32,
    pub held_first: Option<Instant>,
    pub last_held: Option<Instant>,
}

/// What the page is shown of the memory.
#[derive(Debug, Clone, Serialize)]
pub struct Recalled {
    pub id: u32,
    pub phrase: String,
    pub present: bool,
    pub seen_for_s: f32,
    pub gone_for_s: f32,
    pub in_hand: bool,
    /// Seconds since it was last held, when it was.
    pub held_ago_s: Option<f32>,
}

/// A newly observed hand/object relation. The hand remains anonymous.
#[derive(Debug, Clone)]
pub struct HeldEvent {
    pub track_id: u32,
    pub phrase: String,
    pub bbox: [f32; 4],
    pub confidence: f32,
    pub observed_at_ms: i64,
}

#[derive(Default)]
pub struct Memory {
    things: Vec<Thing>,
    next_id: u32,
    /// The ids seen in the latest frame, in the order of its detections.
    present: Vec<u32>,
    held_events: Vec<HeldEvent>,
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    dot / (norm(a) * norm(b))
}

/// The overlap as a share of the smaller box: 1 when one box is inside the other.
fn overlap_of_smaller(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    w * h / area(a).min(area(b)).max(1e-6)
}

fn is_hand(phrase: &str) -> bool {
    matches!(phrase, "hand" | "finger")
}

pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    inter / (area(a) + area(b) - inter).max(1e-6)
}

/// "4 s", "3 min", "2 h": how long ago, said briefly.
pub fn ago(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s / 60),
        _ => format!("{} h", s / 3600),
    }
}

impl Memory {
    /// Takes in a new frame: each thing in it continues the track it matches best, or
    /// starts a new one.
    pub fn update(&mut self, scene: &Scene) {
        let Some(now) = scene.at else {
            return;
        };
        self.present.clear();
        let mut order: Vec<usize> = (0..scene.seen.len()).collect();
        order.sort_by(|a, b| scene.seen[*b].score.total_cmp(&scene.seen[*a].score));
        let mut matched: Vec<u32> = Vec::new();
        for index in order {
            let seen = &scene.seen[index];
            let person = is_person(&seen.phrase);
            let embedding = if person { None } else { seen.embedding.clone() };
            let best = self
                .things
                .iter()
                .enumerate()
                .filter(|(_, thing)| !matched.contains(&thing.id))
                .filter_map(|(at, thing)| {
                    let same_phrase = thing.phrase == seen.phrase;
                    let overlap = iou(&thing.bbox, &seen.bbox);
                    let recent = now.duration_since(thing.last_seen) <= Duration::from_secs(5);
                    let likeness = match (&thing.embedding, &embedding) {
                        (Some(a), Some(b)) => Some(cosine(a, b)),
                        _ => None,
                    };
                    // The same object: it looks the same (IRIS may call it by a near
                    // category from one frame to the next), or it is the same kind of thing
                    // where that kind of thing just was.
                    // Measured on IRIS step 4000: the same object moved in the frame
                    // scored 0.83 to 0.98, different objects in one frame about 0.6 to 0.76.
                    let score = match likeness {
                        Some(like) if same_phrase && like >= 0.85 => like + overlap,
                        Some(like) if like >= 0.93 => like + overlap,
                        _ if same_phrase && recent && overlap >= 0.3 => overlap,
                        _ => return None,
                    };
                    Some((at, score))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(at, _)| at);
            let id = match best {
                Some(at) => {
                    let thing = &mut self.things[at];
                    thing.bbox = seen.bbox;
                    thing.last_seen = now;
                    thing.sightings += 1;
                    thing.best_score = thing.best_score.max(seen.score);
                    if seen.score >= thing.best_score * 0.9 || thing.phrase == seen.phrase {
                        thing.phrase = seen.phrase.clone();
                    }
                    if let (Some(kept), Some(new)) = (&mut thing.embedding, &embedding) {
                        // A slow average: the look of a thing drifts with the light and angle.
                        for (k, n) in kept.iter_mut().zip(new) {
                            *k = 0.8 * *k + 0.2 * n;
                        }
                    } else if thing.embedding.is_none() {
                        thing.embedding = embedding;
                    }
                    thing.id
                }
                None => {
                    self.next_id += 1;
                    self.things.push(Thing {
                        id: self.next_id,
                        phrase: seen.phrase.clone(),
                        bbox: seen.bbox,
                        embedding,
                        first_seen: now,
                        last_seen: now,
                        sightings: 1,
                        best_score: seen.score,
                        touched_frames: 0,
                        held_first: None,
                        last_held: None,
                    });
                    self.next_id
                }
            };
            matched.push(id);
            self.present.push(id);
        }
        let hands: Vec<[f32; 4]> = scene
            .seen
            .iter()
            .filter(|item| is_hand(&item.phrase))
            .map(|item| item.bbox)
            .collect();
        let mut held_events = Vec::new();
        for thing in self.things.iter_mut() {
            let here = thing.last_seen == now && !is_person(&thing.phrase);
            if here
                && hands
                    .iter()
                    .any(|hand| overlap_of_smaller(hand, &thing.bbox) >= TOUCH)
            {
                let was_held = thing.touched_frames >= HELD_AFTER_FRAMES;
                thing.touched_frames += 1;
                if thing.touched_frames >= HELD_AFTER_FRAMES {
                    thing.held_first.get_or_insert(now);
                    thing.last_held = Some(now);
                    if !was_held {
                        if let Some(observed_at_ms) = scene.observed_at_ms {
                            held_events.push(HeldEvent {
                                track_id: thing.id,
                                phrase: thing.phrase.clone(),
                                bbox: thing.bbox,
                                confidence: thing.best_score,
                                observed_at_ms,
                            });
                        }
                    }
                }
            } else if here || now.duration_since(thing.last_seen) >= GONE_AFTER {
                thing.touched_frames = 0;
            }
        }
        self.held_events.extend(held_events);
        self.things.retain(|thing| {
            now.duration_since(thing.last_seen) < FORGET_AFTER
                && (thing.sightings > 1
                    || now.duration_since(thing.last_seen) < Duration::from_secs(60))
        });
        if self.things.len() > CAPACITY {
            self.things.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
            self.things.truncate(CAPACITY);
        }
    }

    /// The track of the `index`-th detection of the latest frame.
    pub fn id_of(&self, index: usize) -> Option<u32> {
        self.present.get(index).copied()
    }

    pub fn take_held_events(&mut self) -> Vec<HeldEvent> {
        std::mem::take(&mut self.held_events)
    }

    fn gone(&self, now: Instant) -> Vec<&Thing> {
        let mut gone: Vec<&Thing> = self
            .things
            .iter()
            .filter(|thing| {
                thing.sightings > 1 && now.duration_since(thing.last_seen) >= GONE_AFTER
            })
            .collect();
        gone.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        gone
    }

    /// What is in a hand now, and what was and is not any more, newest first.
    fn held(&self, scene: &Scene, now: Instant) -> Vec<String> {
        let fresh = scene
            .at
            .is_some_and(|at| now.saturating_duration_since(at) <= Duration::from_secs(4));
        let latest = scene.at;
        let mut holding: Vec<&Thing> = self
            .things
            .iter()
            .filter(|thing| {
                thing
                    .last_held
                    .is_some_and(|at| now.saturating_duration_since(at) <= HELD_TOLD_FOR)
            })
            .collect();
        holding.sort_by(|a, b| b.last_held.cmp(&a.last_held));
        let (now_held, before): (Vec<&Thing>, Vec<&Thing>) = holding
            .into_iter()
            .partition(|thing| fresh && thing.last_held == latest);
        let mut lines = Vec::new();
        if !now_held.is_empty() {
            let names: Vec<&str> = now_held.iter().map(|thing| thing.phrase.as_str()).collect();
            lines.push(format!("En una mano ahora (IRIS): {}.", names.join(", ")));
        }
        let before: Vec<String> = before
            .into_iter()
            .take(6)
            .map(|thing| {
                let held_ago = ago(now.saturating_duration_since(thing.last_held.unwrap_or(now)));
                let still = if fresh && Some(thing.last_seen) == latest {
                    "sigue a la vista"
                } else {
                    "ya no se ve"
                };
                format!("{} (hasta hace {held_ago}; {still})", thing.phrase)
            })
            .collect();
        if !before.is_empty() {
            lines.push(format!(
                "Estuvo en una mano y ya no (lo recuerda IRIS; no sabe de quién era la mano): {}.",
                before.join(", ")
            ));
        }
        lines
    }

    /// The memory for the page: what is seen now, then what was, newest first.
    pub fn recall(&self, now: Instant) -> Vec<Recalled> {
        let present = self
            .present
            .iter()
            .filter_map(|id| self.things.iter().find(|t| t.id == *id));
        present
            .map(|thing| (thing, true))
            .chain(
                self.gone(now)
                    .into_iter()
                    .take(12)
                    .map(|thing| (thing, false)),
            )
            .map(|(thing, present)| Recalled {
                id: thing.id,
                phrase: thing.phrase.clone(),
                present,
                seen_for_s: thing
                    .last_seen
                    .duration_since(thing.first_seen)
                    .as_secs_f32(),
                gone_for_s: if present {
                    0.0
                } else {
                    now.duration_since(thing.last_seen).as_secs_f32()
                },
                in_hand: present
                    && thing.last_held.is_some()
                    && thing.last_held == Some(thing.last_seen),
                held_ago_s: thing
                    .last_held
                    .map(|at| now.saturating_duration_since(at).as_secs_f32()),
            })
            .collect()
    }

    /// What the model is told: what the camera shows now — or that it shows nothing new
    /// — then who is in front of it, then what was there and is not any more.
    pub fn describe(&self, scene: &Scene, now: Instant) -> String {
        let Some(at) = scene.at else {
            return "Cámara: todavía no me llega imagen, así que no sabes qué hay frente a ella."
                .into();
        };
        let age = now.saturating_duration_since(at);
        let mut lines = Vec::new();
        let listed = crate::live::eyes::list(scene);
        if age > Duration::from_secs(4) {
            lines.push(format!(
                "La cámara no manda imagen nueva desde hace {}: no sabes qué hay frente a ella ahora. \
                 Lo último que IRIS vio: {}.",
                ago(age),
                if listed.is_empty() { "nada que reconociera".to_string() } else { listed.join(", ") }
            ));
        } else if listed.is_empty() {
            lines.push("Ahora mismo IRIS no reconoce nada con seguridad en la cámara.".into());
        } else {
            lines.push(format!(
                "Ahora mismo la cámara muestra (IRIS, imagen de hace {:.1} s; son categorías \
                 generales, y las marcadas «quizás» son inseguras): {}.",
                age.as_secs_f32(),
                listed.join(", ")
            ));
        }
        if age <= Duration::from_secs(4) && !scene.seen.iter().any(|item| is_person(&item.phrase)) {
            lines.push("No se ve a ninguna persona frente a la cámara ahora.".into());
        }
        // A person is one entry however many of their parts IRIS boxed, and only while
        // nobody is in view now.
        let someone_here = scene.seen.iter().any(|item| is_person(&item.phrase));
        let mut person_told = someone_here;
        let gone: Vec<String> = self
            .gone(now)
            .into_iter()
            .filter(|thing| !scene.seen.iter().any(|item| item.phrase == thing.phrase))
            .filter_map(|thing| {
                let since = ago(now.duration_since(thing.last_seen));
                if !is_person(&thing.phrase) {
                    return Some(format!("{} (dejó de verse hace {since})", thing.phrase));
                }
                (!std::mem::replace(&mut person_told, true))
                    .then(|| format!("una persona (dejó de verse hace {since})"))
            })
            .take(8)
            .collect();
        if !gone.is_empty() {
            lines.push(format!(
                "Ya no se ven (lo recuerda IRIS; no están ahora): {}.",
                gone.join(", ")
            ));
        }
        lines.extend(self.held(scene, now));
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::eyes::Seen;
    use std::sync::Arc;

    type SceneItem<'a> = (&'a str, [f32; 4], Option<Vec<f32>>);

    fn frame(at: Instant, items: &[SceneItem<'_>]) -> Scene {
        Scene {
            seen: items
                .iter()
                .map(|(phrase, bbox, embedding)| Seen {
                    phrase: phrase.to_string(),
                    score: 0.8,
                    bbox: *bbox,
                    embedding: embedding.clone(),
                })
                .collect(),
            width: 300,
            height: 300,
            frame: Arc::new(Vec::new()),
            at: Some(at),
            observed_at_ms: Some(0),
            step: Some(1),
        }
    }

    #[test]
    fn a_person_who_left_is_not_described_as_still_there() {
        let start = Instant::now();
        let mut memory = Memory::default();
        let with_phone = [
            ("person", [50.0, 0.0, 250.0, 300.0], None),
            (
                "cell phone",
                [120.0, 200.0, 160.0, 260.0],
                Some(vec![1.0, 0.0, 0.0]),
            ),
        ];
        for second in 0..5 {
            memory.update(&frame(start + Duration::from_secs(second), &with_phone));
        }
        let empty_room = |second| {
            frame(
                start + Duration::from_secs(second),
                &[("chair", [0.0, 0.0, 80.0, 80.0], None)],
            )
        };
        for second in 60..63 {
            memory.update(&empty_room(second));
        }
        let told = memory.describe(&empty_room(62), start + Duration::from_secs(62));
        assert!(told.contains("Ahora mismo la cámara muestra"), "{told}");
        assert!(told.contains("chair"), "{told}");
        assert!(told.contains("No se ve a ninguna persona"), "{told}");
        assert!(
            told.contains("cell phone (dejó de verse hace 58 s)"),
            "{told}"
        );
        assert!(
            told.contains("una persona (dejó de verse hace 58 s)"),
            "{told}"
        );
    }

    #[test]
    fn the_same_thing_keeps_its_track_by_how_it_looks_and_where_it_is() {
        let start = Instant::now();
        let mut memory = Memory::default();
        memory.update(&frame(
            start,
            &[("toy", [0.0, 0.0, 50.0, 50.0], Some(vec![1.0, 0.0]))],
        ));
        // Moved across the frame, and called by a near category: the same look.
        memory.update(&frame(
            start + Duration::from_secs(1),
            &[(
                "stuffed animal",
                [200.0, 200.0, 250.0, 250.0],
                Some(vec![0.99, 0.05]),
            )],
        ));
        assert_eq!(memory.id_of(0), Some(1));
        // A different thing that looks different is a new track.
        memory.update(&frame(
            start + Duration::from_secs(2),
            &[("toy", [0.0, 0.0, 50.0, 50.0], Some(vec![0.0, 1.0]))],
        ));
        assert_eq!(memory.id_of(0), Some(2));
    }

    #[test]
    fn a_stale_camera_is_said_to_be_stale() {
        let start = Instant::now();
        let mut memory = Memory::default();
        let scene = frame(start, &[("cup", [0.0, 0.0, 50.0, 50.0], None)]);
        memory.update(&scene);
        let told = memory.describe(&scene, start + Duration::from_secs(20));
        assert!(
            told.contains("no manda imagen nueva desde hace 20 s"),
            "{told}"
        );
        assert!(
            !told.contains("No se ve a ninguna persona"),
            "nothing is claimed about now: {told}"
        );
    }

    #[test]
    fn a_phone_put_down_is_told_as_held_until_a_moment_ago() {
        let start = Instant::now();
        let at = |second| start + Duration::from_secs(second);
        let phone = (
            "cell phone",
            [120.0, 200.0, 160.0, 260.0],
            Some(vec![1.0, 0.0, 0.0]),
        );
        let hand = ("hand", [110.0, 220.0, 170.0, 280.0], None);
        let mut memory = Memory::default();
        for second in 0..3 {
            memory.update(&frame(at(second), &[phone.clone(), hand.clone()]));
        }
        let told = memory.describe(&frame(at(2), &[phone.clone(), hand.clone()]), at(2));
        assert!(
            told.contains("En una mano ahora (IRIS): cell phone."),
            "{told}"
        );
        // Put down on the table: still in view, no hand on it.
        let on_table = (
            "cell phone",
            [20.0, 20.0, 60.0, 80.0],
            Some(vec![1.0, 0.0, 0.0]),
        );
        for second in 3..23 {
            memory.update(&frame(at(second), &[on_table.clone()]));
        }
        let scene = frame(at(22), &[on_table.clone()]);
        let told = memory.describe(&scene, at(22));
        assert!(!told.contains("En una mano ahora"), "{told}");
        assert!(
            told.contains("cell phone (hasta hace 20 s; sigue a la vista)"),
            "{told}"
        );
        let recalled = memory.recall(at(22));
        assert!(!recalled[0].in_hand);
        assert_eq!(recalled[0].held_ago_s, Some(20.0));
    }

    #[test]
    fn a_hand_passing_over_a_thing_for_one_frame_does_not_hold_it() {
        let start = Instant::now();
        let cup = ("cup", [0.0, 0.0, 50.0, 50.0], None);
        let mut memory = Memory::default();
        memory.update(&frame(
            start,
            &[cup.clone(), ("hand", [10.0, 10.0, 40.0, 40.0], None)],
        ));
        for second in 1..4 {
            memory.update(&frame(start + Duration::from_secs(second), &[cup.clone()]));
        }
        let scene = frame(start + Duration::from_secs(3), &[cup.clone()]);
        assert!(!memory
            .describe(&scene, start + Duration::from_secs(3))
            .contains("mano"));
    }

    #[test]
    fn a_thing_seen_in_one_frame_only_is_not_remembered() {
        let start = Instant::now();
        let mut memory = Memory::default();
        memory.update(&frame(start, &[("knife", [0.0, 0.0, 50.0, 50.0], None)]));
        let empty = frame(start + Duration::from_secs(10), &[]);
        memory.update(&empty);
        assert!(!memory
            .describe(&empty, start + Duration::from_secs(10))
            .contains("knife"));
    }
}
