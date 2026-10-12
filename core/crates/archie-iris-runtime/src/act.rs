//! IRIS acting: a frame and an instruction in, a controller state out, with a memory.
//!
//! A port of `act.ActionMemoryHead` and `iris_act.IrisActor` for inference. The memory per
//! session follows the tiers the head was trained on: the instruction or skill (T1), the
//! inputs already given (T2), older frames compacted every half second (T3), the last two
//! frames as state slots (T4), and now. Frames are dropped after the decision; only pooled
//! vectors stay, per session, until it has been idle five minutes.
//!
//! Not here: LoRA skill adapters and recording runs for REM. A call that asks for either
//! is refused rather than served without it.

use crate::detect::round_to;
use crate::model::IrisModel;
use crate::nn::{LayerNorm, Linear, Mlp, MultiheadAttention, Result};
use crate::phrases::PhraseSource;
use crate::preprocess::{letterbox, to_tensor, Rgb};
use crate::weights::Weights;
use crate::Error;
use candle_core::{DType, Tensor, D};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};

pub const KEYS: [&str; 53] = [
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "space",
    "shift",
    "ctrl",
    "alt",
    "tab",
    "esc",
    "enter",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];
pub const MOUSE_BUTTONS: [&str; 3] = ["left", "right", "middle"];
pub const MOUSE_BINS: [i32; 23] = [
    -1000, -500, -300, -200, -100, -60, -30, -20, -10, -4, -2, 0, 2, 4, 10, 20, 30, 60, 100, 200,
    300, 500, 1000,
];
pub const GAMEPAD_AXES: [&str; 6] = [
    "left_x",
    "left_y",
    "right_x",
    "right_y",
    "left_trigger",
    "right_trigger",
];
pub const GAMEPAD_BUTTONS: [&str; 14] = [
    "a",
    "b",
    "x",
    "y",
    "left_bumper",
    "right_bumper",
    "back",
    "start",
    "left_stick",
    "right_stick",
    "dpad_up",
    "dpad_down",
    "dpad_left",
    "dpad_right",
];
pub const STEP_TYPES: [&str; 13] = [
    "none",
    "click",
    "double_click",
    "right_click",
    "long_press",
    "drag",
    "type",
    "scroll",
    "key",
    "back",
    "home",
    "wait",
    "done",
];
pub const SCROLL: [&str; 3] = ["up", "none", "down"];
const ACTION_WIDTH: usize = 53 + 3 + 2 * 23 + 3 + 6 + 14 + 13;

const TIER_DURABLE: usize = 0;
const TIER_EVENT: usize = 1;
const TIER_COMPACTED: usize = 2;
const TIER_RECENT: usize = 3;
const TIER_NOW: usize = 4;
const SOURCE_VISION: usize = 0;
const SOURCE_ACTION: usize = 1;
const SOURCE_PLANNER: usize = 2;

const RECENT: usize = 2;
const COMPACTED: usize = 4;
const COMPACT_EVERY_S: f64 = 0.5;
const EVENTS: usize = 4;
const SESSION_IDLE_S: f64 = 300.0;
const CHUNK_HZ: f64 = 16.0;
const GROUNDING_CONFIDENCE: f32 = 0.3;
const POINTING_STEPS: [&str; 5] = ["click", "double_click", "right_click", "long_press", "drag"];

/// `nn.TransformerEncoderLayer(norm_first=True, activation="gelu")`.
struct EncoderLayer {
    attention: MultiheadAttention,
    norm1: LayerNorm,
    norm2: LayerNorm,
    ffn: Mlp,
}

struct Head {
    future_queries: Option<Tensor>,
    slot_queries: Tensor,
    slot_attention: MultiheadAttention,
    slot_norm: LayerNorm,
    tier: Tensor,
    source: Tensor,
    age: Mlp,
    action_embedding: Linear,
    query: Tensor,
    layers: Vec<EncoderLayer>,
    out_norm: LayerNorm,
    step_type: Linear,
    keys: Linear,
    buttons: Linear,
    mouse_x: Linear,
    mouse_y: Linear,
    scroll: Linear,
    pad_axes: Linear,
    pad_buttons: Linear,
    pointer_query: Linear,
    pointer_key: Linear,
    pointer_refine: Linear,
}

/// The memory of one decision, embedded: `[1, L, D]` each, with ages in seconds.
struct Memory {
    durable: Tensor,
    events: Option<(Tensor, Vec<f64>)>,
    compacted: Option<(Tensor, Vec<f64>)>,
    recent: Option<(Tensor, Vec<f64>)>,
}

struct Outputs {
    controls: HashMap<&'static str, Vec<f32>>,
    pointer: [f32; 2],
    future: Vec<HashMap<&'static str, Vec<f32>>>,
}

impl Head {
    fn load(weights: &Weights, heads: usize, layers: usize) -> Result<Self> {
        let l = |name: &str| Linear::load(weights, &format!("act.{name}"));
        let layers = (0..layers)
            .map(|i| {
                let p = format!("act.memory.layers.{i}");
                Ok(EncoderLayer {
                    attention: MultiheadAttention::load(weights, &format!("{p}.self_attn"), heads)?,
                    norm1: LayerNorm::load(weights, &format!("{p}.norm1"), 1e-5)?,
                    norm2: LayerNorm::load(weights, &format!("{p}.norm2"), 1e-5)?,
                    ffn: Mlp::load(
                        weights,
                        &format!("{p}.linear1"),
                        &format!("{p}.linear2"),
                        true,
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            future_queries: if weights.contains("act.future_queries") {
                Some(weights.get("act.future_queries")?)
            } else {
                None
            },
            slot_queries: weights.get("act.slot_queries")?,
            slot_attention: MultiheadAttention::load(weights, "act.slot_attention", heads)?,
            slot_norm: LayerNorm::load(weights, "act.slot_norm", 1e-5)?,
            tier: weights.get("act.tier.weight")?,
            source: weights.get("act.source.weight")?,
            age: Mlp::load(weights, "act.age.0", "act.age.2", true)?,
            action_embedding: l("action_embedding")?,
            query: weights.get("act.query")?,
            layers,
            out_norm: LayerNorm::load(weights, "act.out_norm", 1e-5)?,
            step_type: l("step_type")?,
            keys: l("keys")?,
            buttons: l("buttons")?,
            mouse_x: l("mouse_x")?,
            mouse_y: l("mouse_y")?,
            scroll: l("scroll")?,
            pad_axes: l("pad_axes")?,
            pad_buttons: l("pad_buttons")?,
            pointer_query: l("pointer_query")?,
            pointer_key: l("pointer_key")?,
            pointer_refine: l("pointer_refine")?,
        })
    }

    /// A frame's fused tokens `[1, N, D]` as state slots `[1, S, D]`.
    fn pool(&self, fused: &Tensor) -> Result<Tensor> {
        let queries = self.slot_queries.unsqueeze(0)?;
        let slots = self.slot_attention.forward(&queries, fused, fused, None)?;
        self.slot_norm.forward(&(slots + queries)?)
    }

    fn stamp(&self, values: &Tensor, ages: &[f64], tier: usize, source: usize) -> Result<Tensor> {
        let device = values.device();
        let dtype = values.dtype();
        let features: Vec<f32> = ages
            .iter()
            .flat_map(|&a| {
                let a = a.max(0.0) as f32;
                [a, a.ln_1p()]
            })
            .collect();
        let features = Tensor::from_vec(features, (1, ages.len(), 2), device)?.to_dtype(dtype)?;
        let stamp = self.age.forward(&features)?;
        let tier = self.tier.get(tier)?;
        let source = self.source.get(source)?;
        Ok(values
            .broadcast_add(&stamp)?
            .broadcast_add(&tier)?
            .broadcast_add(&source)?)
    }

    fn controls(&self, out: &Tensor) -> Result<HashMap<&'static str, Vec<f32>>> {
        let row = |linear: &Linear| -> Result<Vec<f32>> {
            Ok(linear
                .forward(out)?
                .to_dtype(DType::F32)?
                .flatten_all()?
                .to_vec1()?)
        };
        let mut map = HashMap::new();
        map.insert("step_type", row(&self.step_type)?);
        map.insert("keys", row(&self.keys)?);
        map.insert("buttons", row(&self.buttons)?);
        map.insert("mouse_x", row(&self.mouse_x)?);
        map.insert("mouse_y", row(&self.mouse_y)?);
        map.insert("scroll", row(&self.scroll)?);
        map.insert(
            "pad_axes",
            row(&self.pad_axes)?.into_iter().map(f32::tanh).collect(),
        );
        map.insert("pad_buttons", row(&self.pad_buttons)?);
        Ok(map)
    }

    fn forward(&self, now_fused: &Tensor, anchors: &Tensor, memory: &Memory) -> Result<Outputs> {
        let d = now_fused.dim(D::Minus1)?;
        let now_slots = self.pool(now_fused)?;
        let mut parts = vec![self.stamp(
            &memory.durable,
            &vec![0.0; memory.durable.dim(1)?],
            TIER_DURABLE,
            SOURCE_PLANNER,
        )?];
        if let Some((events, ages)) = &memory.events {
            parts.push(self.stamp(events, ages, TIER_EVENT, SOURCE_ACTION)?);
        }
        if let Some((compacted, ages)) = &memory.compacted {
            parts.push(self.stamp(compacted, ages, TIER_COMPACTED, SOURCE_VISION)?);
        }
        if let Some((recent, ages)) = &memory.recent {
            let slots = recent.dim(1)? / ages.len();
            let expanded: Vec<f64> = ages
                .iter()
                .flat_map(|&a| std::iter::repeat_n(a, slots))
                .collect();
            parts.push(self.stamp(recent, &expanded, TIER_RECENT, SOURCE_VISION)?);
        }
        parts.push(self.stamp(
            &now_slots,
            &vec![0.0; now_slots.dim(1)?],
            TIER_NOW,
            SOURCE_VISION,
        )?);
        parts.push(self.query.clone());
        let future = self
            .future_queries
            .as_ref()
            .map_or(0, |f| f.dim(0).unwrap_or(0));
        if let Some(queries) = &self.future_queries {
            parts.push(queries.unsqueeze(0)?);
        }
        let mut sequence = Tensor::cat(&parts, 1)?;
        let length = sequence.dim(1)?;
        // Nothing else sees the future queries; they see everything.
        let allowed = if future > 0 {
            let mut mask = vec![1u8; length * length];
            for row in 0..length - future {
                for column in length - future..length {
                    mask[row * length + column] = 0;
                }
            }
            Some(Tensor::from_vec(
                mask,
                (1, 1, length, length),
                sequence.device(),
            )?)
        } else {
            None
        };
        for layer in &self.layers {
            let normed = layer.norm1.forward(&sequence)?;
            sequence = (&sequence
                + layer
                    .attention
                    .forward(&normed, &normed, &normed, allowed.as_ref())?)?;
            sequence = (&sequence + layer.ffn.forward(&layer.norm2.forward(&sequence)?)?)?;
        }
        let out = self
            .out_norm
            .forward(&sequence.narrow(1, length - 1 - future, 1)?)?
            .squeeze(1)?;
        let scores = self
            .pointer_query
            .forward(&out)?
            .unsqueeze(1)?
            .broadcast_mul(&self.pointer_key.forward(now_fused)?)?
            .sum(D::Minus1)?
            .to_dtype(DType::F32)?;
        let weights = candle_nn::ops::softmax_last_dim(&(scores / (d as f64).sqrt())?)?;
        let centres = anchors.narrow(1, 0, 2)?.to_dtype(DType::F32)?;
        let pointer: Vec<f32> = weights.matmul(&centres)?.flatten_all()?.to_vec1()?;
        let refine: Vec<f32> = self
            .pointer_refine
            .forward(&out)?
            .to_dtype(DType::F32)?
            .flatten_all()?
            .to_vec1()?;
        let pointer = [
            (pointer[0] + 0.05 * refine[0].tanh()).clamp(0.0, 1.0),
            (pointer[1] + 0.05 * refine[1].tanh()).clamp(0.0, 1.0),
        ];
        let controls = self.controls(&out)?;
        let mut planned = Vec::with_capacity(future);
        if future > 0 {
            let tail = self
                .out_norm
                .forward(&sequence.narrow(1, length - future, future)?)?;
            for step in 0..future {
                planned.push(self.controls(&tail.narrow(1, step, 1)?.squeeze(1)?)?);
            }
        }
        Ok(Outputs {
            controls,
            pointer,
            future: planned,
        })
    }

    fn embed_actions(
        &self,
        vectors: &[Vec<f32>],
        device: &candle_core::Device,
        dtype: DType,
    ) -> Result<Tensor> {
        let flat: Vec<f32> = vectors.iter().flatten().copied().collect();
        let tensor =
            Tensor::from_vec(flat, (1, vectors.len(), ACTION_WIDTH), device)?.to_dtype(dtype)?;
        self.action_embedding.forward(&tensor)
    }
}

fn argmax(values: &[f32]) -> usize {
    let mut best = 0;
    for (index, value) in values.iter().enumerate() {
        if *value > values[best] {
            best = index;
        }
    }
    best
}

fn held<const N: usize>(logits: &[f32], names: [&str; N]) -> Vec<String> {
    logits
        .iter()
        .zip(names)
        .filter(|(logit, _)| 1.0 / (1.0 + (-**logit).exp()) > 0.5)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// `decode_control`: one frame's output as named controller state.
fn decode(controls: &HashMap<&'static str, Vec<f32>>, pointer: Option<[f32; 2]>) -> Value {
    let axes: serde_json::Map<String, Value> = GAMEPAD_AXES
        .iter()
        .zip(&controls["pad_axes"])
        .map(|(name, value)| ((*name).to_owned(), json!(round_to(f64::from(*value), 3))))
        .collect();
    let mut out = json!({
        "step_type": STEP_TYPES[argmax(&controls["step_type"])],
        "keys": held(&controls["keys"], KEYS),
        "buttons": held(&controls["buttons"], MOUSE_BUTTONS),
        "mouse": [MOUSE_BINS[argmax(&controls["mouse_x"])], MOUSE_BINS[argmax(&controls["mouse_y"])]],
        "scroll": SCROLL[argmax(&controls["scroll"])],
        "pad_axes": axes,
        "pad_buttons": held(&controls["pad_buttons"], GAMEPAD_BUTTONS),
    });
    if let Some(p) = pointer {
        out["pointer_canvas"] = json!([round_to(f64::from(p[0]), 4), round_to(f64::from(p[1]), 4)]);
    }
    out
}

/// `action_vector(control_target(control))`: what the memory keeps of an output.
fn action_vector(control: &Value) -> Vec<f32> {
    let mut vector = vec![0f32; ACTION_WIDTH];
    let index_of = |names: &[&str], name: &str| names.iter().position(|n| *n == name);
    let mut offset = 0;
    for key in control["keys"].as_array().into_iter().flatten() {
        if let Some(i) = index_of(&KEYS, key.as_str().unwrap_or_default()) {
            vector[offset + i] = 1.0;
        }
    }
    offset += KEYS.len();
    for button in control["buttons"].as_array().into_iter().flatten() {
        if let Some(i) = index_of(&MOUSE_BUTTONS, button.as_str().unwrap_or_default()) {
            vector[offset + i] = 1.0;
        }
    }
    offset += MOUSE_BUTTONS.len();
    for axis in 0..2 {
        let bin = control["mouse"][axis].as_i64().unwrap_or(0) as i32;
        let i = MOUSE_BINS.iter().position(|&b| b == bin).unwrap_or(11);
        vector[offset + i] = 1.0;
        offset += MOUSE_BINS.len();
    }
    vector[offset + index_of(&SCROLL, control["scroll"].as_str().unwrap_or("none")).unwrap_or(1)] =
        1.0;
    offset += SCROLL.len();
    for (i, name) in GAMEPAD_AXES.iter().enumerate() {
        vector[offset + i] = control["pad_axes"][*name].as_f64().unwrap_or(0.0) as f32;
    }
    offset += GAMEPAD_AXES.len();
    for button in control["pad_buttons"].as_array().into_iter().flatten() {
        if let Some(i) = index_of(&GAMEPAD_BUTTONS, button.as_str().unwrap_or_default()) {
            vector[offset + i] = 1.0;
        }
    }
    offset += GAMEPAD_BUTTONS.len();
    vector[offset
        + index_of(&STEP_TYPES, control["step_type"].as_str().unwrap_or("none")).unwrap_or(0)] =
        1.0;
    vector
}

#[derive(Default)]
struct Session {
    recent: VecDeque<(Tensor, f64)>,
    compacted: VecDeque<(Tensor, f64)>,
    events: VecDeque<(Vec<f32>, f64)>,
    last_compacted: f64,
    last_seen: f64,
}

pub struct Actor {
    head: Head,
    sessions: HashMap<String, Session>,
}

impl Actor {
    pub fn load(weights: &Weights) -> Result<Self> {
        let config = &weights.release.config;
        if !config.act_head {
            return Err(Error::Weights("these weights have no action head".into()));
        }
        Ok(Self {
            head: Head::load(weights, config.heads, config.act_layers)?,
            sessions: HashMap::new(),
        })
    }

    pub fn reset(&mut self, session: &str) {
        self.sessions.remove(session);
    }

    /// One frame of acting. `now` is seconds on a monotonic clock.
    #[allow(clippy::too_many_arguments)]
    pub fn act(
        &mut self,
        model: &IrisModel,
        phrases: &mut dyn PhraseSource,
        session_name: &str,
        image: &Rgb,
        instruction: &str,
        skill: &str,
        now: f64,
    ) -> Result<Value> {
        let size = model.config.image_size;
        let (canvas, scale) = letterbox(image, size);
        let tokens = model.encode_image(&to_tensor(&canvas, &model.device)?)?;
        let instruction_features = phrases.encode(&[instruction.to_owned()])?;
        let encoded = model.encode_phrases(&[instruction_features])?;
        // The scene with no phrase, as memory frames were in training: the null token padded
        // to the instruction's length, both fused as one batch of two.
        let length = encoded.text.dim(1)?;
        let d = model.config.dimension;
        let null_row = Tensor::cat(
            &[
                model.null_phrase.reshape((1, d))?,
                Tensor::zeros((length - 1, d), model.dtype, &model.device)?,
            ],
            0,
        )?
        .unsqueeze(0)?;
        let mut null_padding = vec![1u8; length];
        null_padding[0] = 0;
        let null_padding = Tensor::from_vec(null_padding, (1, length), &model.device)?;
        let text = Tensor::cat(&[&encoded.text, &null_row], 0)?;
        let padding = Tensor::cat(&[&encoded.text_padding, &null_padding], 0)?;
        let both = tokens.broadcast_as((2, tokens.dim(1)?, d))?.contiguous()?;
        let (fused_both, text_both) = model.fuse(&both, &text, &padding)?;
        let fused = fused_both.narrow(0, 0, 1)?;
        let fused_null = fused_both.narrow(0, 1, 1)?;

        let durable_text = if skill.is_empty() { instruction } else { skill };
        let durable_encoded =
            model.encode_phrases(&[phrases.encode(&[durable_text.to_owned()])?])?;
        let durable = durable_encoded.vectors.narrow(1, 0, 1)?;
        let head = &self.head;
        let session = {
            self.sessions
                .retain(|_, s| now - s.last_seen <= SESSION_IDLE_S);
            let s = self
                .sessions
                .entry(session_name.to_owned())
                .or_insert_with(|| Session {
                    last_compacted: -1e9,
                    ..Session::default()
                });
            s.last_seen = now;
            s
        };
        let ages = |items: &VecDeque<(Tensor, f64)>| {
            items.iter().map(|(_, t)| now - t).collect::<Vec<f64>>()
        };
        let memory = Memory {
            durable,
            events: if session.events.is_empty() {
                None
            } else {
                let vectors: Vec<Vec<f32>> =
                    session.events.iter().map(|(v, _)| v.clone()).collect();
                Some((
                    head.embed_actions(&vectors, &model.device, model.dtype)?,
                    session.events.iter().map(|(_, t)| now - t).collect(),
                ))
            },
            compacted: if session.compacted.is_empty() {
                None
            } else {
                let rows: Vec<Tensor> = session.compacted.iter().map(|(t, _)| t.clone()).collect();
                Some((
                    Tensor::stack(&rows, 0)?.unsqueeze(0)?,
                    ages(&session.compacted),
                ))
            },
            recent: if session.recent.is_empty() {
                None
            } else {
                let rows: Vec<Tensor> = session.recent.iter().map(|(t, _)| t.clone()).collect();
                Some((Tensor::cat(&rows, 0)?.unsqueeze(0)?, ages(&session.recent)))
            },
        };
        let anchors = model.anchors.clone();
        let out = head.forward(&fused, &anchors, &memory)?;
        let mut control = decode(&out.controls, Some(out.pointer));
        control["pointer_source"] = json!("act");
        let step = control["step_type"].as_str().unwrap_or_default().to_owned();
        if POINTING_STEPS.contains(&step.as_str()) {
            let grounded = model.decode(&fused, &text_both.narrow(0, 0, 1)?, &encoded)?;
            let scores: Vec<f32> =
                candle_nn::ops::sigmoid(&grounded.logits.get(0)?.narrow(1, 0, 1)?)?
                    .flatten_all()?
                    .to_vec1()?;
            let best = argmax(&scores);
            if scores[best] >= GROUNDING_CONFIDENCE {
                let centre: Vec<f32> = grounded
                    .boxes
                    .get(0)?
                    .get(best)?
                    .to_dtype(DType::F32)?
                    .to_vec1()?;
                control["pointer_canvas"] = json!([
                    round_to(f64::from(centre[0].clamp(0.0, 1.0)), 4),
                    round_to(f64::from(centre[1].clamp(0.0, 1.0)), 4)
                ]);
                control["pointer_source"] = json!("grounding");
                control["grounding_score"] = json!(round_to(f64::from(scores[best]), 3));
            }
        }
        if !out.future.is_empty() {
            control["chunk"] = Value::Array(out.future.iter().map(|c| decode(c, None)).collect());
            control["chunk_hz"] = json!(CHUNK_HZ);
        }
        let slots = head.pool(&fused_null)?.squeeze(0)?;
        if session.recent.len() == RECENT && now - session.last_compacted >= COMPACT_EVERY_S {
            let (oldest, when) = session.recent[0].clone();
            session.compacted.push_back((oldest.mean(0)?, when));
            if session.compacted.len() > COMPACTED {
                session.compacted.pop_front();
            }
            session.last_compacted = now;
        }
        session.recent.push_back((slots, now));
        if session.recent.len() > RECENT {
            session.recent.pop_front();
        }
        session.events.push_back((action_vector(&control), now));
        if session.events.len() > EVENTS {
            session.events.pop_front();
        }
        let x = control["pointer_canvas"][0].as_f64().unwrap_or(0.0);
        let y = control["pointer_canvas"][1].as_f64().unwrap_or(0.0);
        control["pointer"] = json!([
            round_to(x * size as f64 / scale, 1),
            round_to(y * size as f64 / scale, 1)
        ]);
        control["memory"] = json!({"recent": session.recent.len(), "compacted": session.compacted.len(), "events": session.events.len()});
        control["adapter"] = Value::Null;
        Ok(control)
    }
}
