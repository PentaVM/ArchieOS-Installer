//! The detector itself: three small convolutions and a head, trained here in Rust.
//!
//! It is deliberately tiny — about five thousand numbers — because the job is small. It
//! looks at a 160x96 grey tile and, for each 8x8 cell of it, says whether the centre of a
//! control sits there, where its box is, and which of three kinds it is. Nothing in it
//! reads, and nothing in it decides.
//!
//! Training is plain stochastic gradient descent with momentum, written out rather than
//! pulled in, and seeded, so the same corpus gives the same model on any machine.

use crate::{Box2, ControlKind, Sample, TILE_HEIGHT, TILE_WIDTH};

/// The grid the tile is divided into: one prediction per 8x8 cell.
pub const CELL: usize = 8;
pub const GRID_WIDTH: usize = TILE_WIDTH / CELL;
pub const GRID_HEIGHT: usize = TILE_HEIGHT / CELL;
/// Objectness, four numbers of box, and one logit per kind.
pub const OUTPUTS: usize = 5 + ControlKind::COUNT;
/// How many controls one window is allowed to teach or to hold.
pub const MAX_CONTROLS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
struct Tensor {
    channels: usize,
    height: usize,
    width: usize,
    data: Vec<f32>,
}

impl Tensor {
    fn zeros(channels: usize, height: usize, width: usize) -> Self {
        Self {
            channels,
            height,
            width,
            data: vec![0.0; channels * height * width],
        }
    }

    #[inline]
    fn at(&self, channel: usize, y: usize, x: usize) -> usize {
        (channel * self.height + y) * self.width + x
    }
}

/// One convolution: a square kernel, a stride, and one pixel of padding.
#[derive(Debug, Clone, PartialEq)]
pub struct Conv {
    pub in_channels: usize,
    pub out_channels: usize,
    pub kernel: usize,
    pub stride: usize,
    pub pad: usize,
    pub weights: Vec<f32>,
    pub biases: Vec<f32>,
}

impl Conv {
    fn new(
        in_channels: usize,
        out_channels: usize,
        kernel: usize,
        stride: usize,
        pad: usize,
        random: &mut Random,
    ) -> Self {
        // Small random weights, scaled by how many numbers each output sums, so the first
        // pass through the network neither vanishes nor explodes.
        let fan_in = (in_channels * kernel * kernel) as f32;
        let spread = (2.0 / fan_in).sqrt();
        let weights = (0..out_channels * in_channels * kernel * kernel)
            .map(|_| random.normal() * spread)
            .collect();
        Self {
            in_channels,
            out_channels,
            kernel,
            stride,
            pad,
            weights,
            biases: vec![0.0; out_channels],
        }
    }

    fn out_size(&self, height: usize, width: usize) -> (usize, usize) {
        (
            (height + 2 * self.pad - self.kernel) / self.stride + 1,
            (width + 2 * self.pad - self.kernel) / self.stride + 1,
        )
    }

    fn forward(&self, input: &Tensor) -> Tensor {
        let (out_h, out_w) = self.out_size(input.height, input.width);
        let mut out = Tensor::zeros(self.out_channels, out_h, out_w);
        for oc in 0..self.out_channels {
            for oy in 0..out_h {
                for ox in 0..out_w {
                    let mut sum = self.biases[oc];
                    for ic in 0..self.in_channels {
                        for ky in 0..self.kernel {
                            let iy = oy * self.stride + ky;
                            if iy < self.pad || iy - self.pad >= input.height {
                                continue;
                            }
                            let iy = iy - self.pad;
                            for kx in 0..self.kernel {
                                let ix = ox * self.stride + kx;
                                if ix < self.pad || ix - self.pad >= input.width {
                                    continue;
                                }
                                let ix = ix - self.pad;
                                let weight =
                                    self.weights[((oc * self.in_channels + ic) * self.kernel + ky)
                                        * self.kernel
                                        + kx];
                                sum += weight * input.data[input.at(ic, iy, ix)];
                            }
                        }
                    }
                    let at = out.at(oc, oy, ox);
                    out.data[at] = sum;
                }
            }
        }
        out
    }

    /// The gradient of the loss with respect to this layer's input, adding what it says
    /// about the weights into `grads`.
    fn backward(&self, input: &Tensor, grad_out: &Tensor, grads: &mut Gradients) -> Tensor {
        let mut grad_in = Tensor::zeros(input.channels, input.height, input.width);
        for oc in 0..self.out_channels {
            for oy in 0..grad_out.height {
                for ox in 0..grad_out.width {
                    let delta = grad_out.data[grad_out.at(oc, oy, ox)];
                    if delta == 0.0 {
                        continue;
                    }
                    grads.biases[oc] += delta;
                    for ic in 0..self.in_channels {
                        for ky in 0..self.kernel {
                            let iy = oy * self.stride + ky;
                            if iy < self.pad || iy - self.pad >= input.height {
                                continue;
                            }
                            let iy = iy - self.pad;
                            for kx in 0..self.kernel {
                                let ix = ox * self.stride + kx;
                                if ix < self.pad || ix - self.pad >= input.width {
                                    continue;
                                }
                                let ix = ix - self.pad;
                                let index = ((oc * self.in_channels + ic) * self.kernel + ky)
                                    * self.kernel
                                    + kx;
                                let value = input.data[input.at(ic, iy, ix)];
                                grads.weights[index] += delta * value;
                                let into = grad_in.at(ic, iy, ix);
                                grad_in.data[into] += delta * self.weights[index];
                            }
                        }
                    }
                }
            }
        }
        grad_in
    }
}

struct Gradients {
    weights: Vec<f32>,
    biases: Vec<f32>,
}

impl Gradients {
    fn like(conv: &Conv) -> Self {
        Self {
            weights: vec![0.0; conv.weights.len()],
            biases: vec![0.0; conv.biases.len()],
        }
    }

    fn clear(&mut self) {
        self.weights.iter_mut().for_each(|value| *value = 0.0);
        self.biases.iter_mut().for_each(|value| *value = 0.0);
    }
}

/// A deterministic little generator: the same seed gives the same model.
pub struct Random(u64);

impl Random {
    pub fn seeded(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn uniform(&mut self) -> f32 {
        (self.next() >> 11) as f32 / (1_u64 << 53) as f32
    }

    /// Two uniforms make one normal, which is all the shape the weights need.
    fn normal(&mut self) -> f32 {
        let (a, b) = (self.uniform().max(1e-7), self.uniform());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }
}

/// How much of a negative number a rectifier still passes. Without this a unit that is
/// pushed under zero everywhere receives no gradient ever again and is gone for good; a
/// training run that loses most of its units this way settles for saying the same thing
/// about every cell, which the loss alone can look content with.
const LEAK: f32 = 0.1;

fn rectify(value: f32) -> f32 {
    if value < 0.0 {
        value * LEAK
    } else {
        value
    }
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-30.0, 30.0)).exp())
}

/// What the model says about one control it found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prediction {
    pub control: Box2,
    /// How sure it is that something is there, in permille.
    pub confidence_permille: u16,
    /// How unsure it is about which kind it is, in permille. IRIS's runtime escalates on
    /// this, which is the point: an uncertain reflex asks for a better look.
    pub uncertainty_permille: u16,
}

/// The detector: three convolutions that make the picture smaller and deeper, and a head
/// that turns each cell of what is left into one answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Detector {
    pub layers: Vec<Conv>,
}

impl Detector {
    pub fn new(seed: u64) -> Self {
        let mut random = Random::seeded(seed);
        Self {
            layers: vec![
                Conv::new(1, 8, 3, 2, 1, &mut random),
                Conv::new(8, 16, 3, 2, 1, &mut random),
                Conv::new(16, 24, 3, 2, 1, &mut random),
                Conv::new(24, OUTPUTS, 1, 1, 0, &mut random),
            ],
        }
    }

    /// How many numbers the whole thing is.
    pub fn parameters(&self) -> usize {
        self.layers
            .iter()
            .map(|layer| layer.weights.len() + layer.biases.len())
            .sum()
    }

    fn input_of(pixels: &[u8]) -> Tensor {
        Tensor {
            channels: 1,
            height: TILE_HEIGHT,
            width: TILE_WIDTH,
            // Greys as they are, centred, so that a mid-grey window is not a large number.
            data: pixels
                .iter()
                .map(|value| (f32::from(*value) - 128.0) / 128.0)
                .collect(),
        }
    }

    /// Everything the network computes on the way, kept for the backward pass.
    fn run(&self, input: &Tensor) -> Vec<Tensor> {
        let mut activations = vec![input.clone()];
        for (index, layer) in self.layers.iter().enumerate() {
            let mut out = layer.forward(activations.last().expect("an input"));
            // Rectified in between, plain at the head, where the numbers mean things.
            if index + 1 < self.layers.len() {
                out.data
                    .iter_mut()
                    .for_each(|value| *value = rectify(*value));
            }
            activations.push(out);
        }
        activations
    }

    /// What the model says about one tile, in the tile's own coordinates.
    pub fn detect(&self, pixels: &[u8], least_confidence_permille: u16) -> Vec<Prediction> {
        let activations = self.run(&Self::input_of(pixels));
        let head = activations.last().expect("a head");
        let mut found = Vec::new();
        for cy in 0..GRID_HEIGHT {
            for cx in 0..GRID_WIDTH {
                let read = |channel: usize| head.data[head.at(channel, cy, cx)];
                let confidence = sigmoid(read(0));
                let permille = (confidence * 1_000.0).round().clamp(0.0, 1_000.0) as u16;
                if permille < least_confidence_permille {
                    continue;
                }
                let (dx, dy) = (sigmoid(read(1)), sigmoid(read(2)));
                let (w, h) = (sigmoid(read(3)), sigmoid(read(4)));
                let centre_x = (cx as f32 + dx) * CELL as f32;
                let centre_y = (cy as f32 + dy) * CELL as f32;
                let width = (w * TILE_WIDTH as f32).max(1.0);
                let height = (h * TILE_HEIGHT as f32).max(1.0);
                let kinds: Vec<f32> = (0..ControlKind::COUNT).map(|k| read(5 + k)).collect();
                let (best, runner) = two_largest(&kinds);
                let sure = softmax_gap(&kinds, best, runner);
                found.push(Prediction {
                    control: Box2 {
                        kind: ControlKind::from_index(best).unwrap_or(ControlKind::Other),
                        x: centre_x - width / 2.0,
                        y: centre_y - height / 2.0,
                        width,
                        height,
                    },
                    confidence_permille: permille,
                    uncertainty_permille: 1_000_u16.saturating_sub(sure),
                });
            }
        }
        found.sort_by_key(|found| std::cmp::Reverse(found.confidence_permille));
        found.truncate(MAX_CONTROLS);
        suppress_overlaps(found)
    }
}

fn two_largest(values: &[f32]) -> (usize, usize) {
    let mut best = 0;
    for (index, value) in values.iter().enumerate() {
        if *value > values[best] {
            best = index;
        }
    }
    let mut runner = usize::from(best == 0);
    for (index, value) in values.iter().enumerate() {
        if index != best && *value > values[runner] {
            runner = index;
        }
    }
    (best, runner)
}

/// How sure the kind is, as the softmax distance between the best and the next, in permille.
fn softmax_gap(values: &[f32], best: usize, runner: usize) -> u16 {
    let top = values[best];
    let total: f32 = values.iter().map(|value| (value - top).exp()).sum();
    let first = 1.0 / total;
    let second = (values[runner] - top).exp() / total;
    ((first - second).clamp(0.0, 1.0) * 1_000.0) as u16
}

/// Two answers about the same control are one answer: the surer one.
fn suppress_overlaps(found: Vec<Prediction>) -> Vec<Prediction> {
    let mut kept: Vec<Prediction> = Vec::new();
    for candidate in found {
        let overlaps = kept
            .iter()
            .any(|other| overlap(&candidate.control, &other.control) > 0.4);
        if !overlaps {
            kept.push(candidate);
        }
    }
    kept
}

/// The share of the smaller box that the two share.
fn overlap(a: &Box2, b: &Box2) -> f32 {
    let x = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let y = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if x <= 0.0 || y <= 0.0 {
        return 0.0;
    }
    let shared = x * y;
    let smaller = (a.width * a.height).min(b.width * b.height).max(1.0);
    shared / smaller
}

/// How a training run is set up. The defaults are what the corpus in this repository was
/// trained with.
#[derive(Debug, Clone, Copy)]
pub struct Training {
    pub epochs: usize,
    pub learning_rate: f32,
    pub momentum: f32,
    /// How much more a cell with a control in it counts than one without. Most cells are
    /// empty, and without this the model would learn to say "nothing" and be mostly right.
    pub presence_weight: f32,
    pub seed: u64,
    /// The share of the corpus kept back to be judged on.
    pub held_back: f32,
}

impl Default for Training {
    fn default() -> Self {
        Self {
            epochs: 120,
            // Ten times smaller than the first run of this trainer used, which learned for
            // one epoch and then killed most of its own units. See `alive` in `Epoch`.
            learning_rate: 0.002,
            momentum: 0.9,
            presence_weight: 12.0,
            seed: 20_260_917,
            held_back: 0.2,
        }
    }
}

impl Training {
    /// The corpus in the order it is taught, and where the part kept back to be judged on
    /// begins. The shuffle comes before the split, so the held-back part is not simply the
    /// last windows collected, which would all come from one run of one application. The
    /// same seed always gives the same split, so a score is about windows the model that
    /// earned it never saw.
    pub fn split(&self, samples: usize) -> (Vec<usize>, usize) {
        let mut random = Random::seeded(self.seed ^ 0x5eed);
        let mut order: Vec<usize> = (0..samples).collect();
        for index in (1..order.len()).rev() {
            let other = (random.next() % (index as u64 + 1)) as usize;
            order.swap(index, other);
        }
        let held_back = ((samples as f32 * self.held_back) as usize).min(samples / 2);
        (order, samples - held_back)
    }
}

/// What a training run looked like, one epoch at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Epoch {
    pub epoch: usize,
    pub loss: f32,
    /// On the part of the corpus the model never trained on.
    pub held_back_loss: f32,
    pub found_permille: u16,
    pub wrong_permille: u16,
    /// How much the model's answer varies from cell to cell, in permille of a full
    /// sigmoid. A model that has collapsed to the corpus's prior — saying the same thing
    /// everywhere, which the loss alone can look content with — reads near zero here.
    pub spread_permille: u16,
    /// The share of the hidden channels that fire anywhere at all. Rectifiers pushed far
    /// under zero everywhere learn very slowly even leaking, so this is worth watching.
    pub alive_permille: u16,
}

/// One cell's worth of what the model should have said.
struct Target {
    cell_x: usize,
    cell_y: usize,
    dx: f32,
    dy: f32,
    width: f32,
    height: f32,
    kind: usize,
}

fn targets_of(controls: &[Box2]) -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();
    for control in controls.iter().take(MAX_CONTROLS) {
        let (centre_x, centre_y) = control.centre();
        let cell_x = ((centre_x / CELL as f32) as usize).min(GRID_WIDTH - 1);
        let cell_y = ((centre_y / CELL as f32) as usize).min(GRID_HEIGHT - 1);
        // Two controls whose centres share a cell: the smaller one is the one to learn,
        // because a large box around a small control is the mistake that misses clicks.
        if let Some(existing) = targets
            .iter_mut()
            .find(|target| target.cell_x == cell_x && target.cell_y == cell_y)
        {
            if control.width * control.height
                < existing.width * TILE_WIDTH as f32 * (existing.height * TILE_HEIGHT as f32)
            {
                existing.dx = centre_x / CELL as f32 - cell_x as f32;
                existing.dy = centre_y / CELL as f32 - cell_y as f32;
                existing.width = control.width / TILE_WIDTH as f32;
                existing.height = control.height / TILE_HEIGHT as f32;
                existing.kind = control.kind.index();
            }
            continue;
        }
        targets.push(Target {
            cell_x,
            cell_y,
            dx: centre_x / CELL as f32 - cell_x as f32,
            dy: centre_y / CELL as f32 - cell_y as f32,
            width: control.width / TILE_WIDTH as f32,
            height: control.height / TILE_HEIGHT as f32,
            kind: control.kind.index(),
        });
    }
    targets
}

/// The loss of one tile, and the gradient at the head that goes with it.
fn head_gradient(
    head: &Tensor,
    targets: &[Target],
    training: &Training,
) -> (f32, Tensor, u32, u32) {
    let mut grad = Tensor::zeros(head.channels, head.height, head.width);
    let mut loss = 0.0;
    let (mut found, mut missed) = (0_u32, 0_u32);
    let cells = (GRID_WIDTH * GRID_HEIGHT) as f32;
    for cy in 0..GRID_HEIGHT {
        for cx in 0..GRID_WIDTH {
            let target = targets
                .iter()
                .find(|target| target.cell_x == cx && target.cell_y == cy);
            let present = target.is_some();
            let logit = head.data[head.at(0, cy, cx)];
            let said = sigmoid(logit);
            let weight = if present {
                training.presence_weight
            } else {
                1.0
            };
            // Cross entropy on whether something is there, weighted towards the few cells
            // where something is.
            let wanted = if present { 1.0 } else { 0.0 };
            loss += weight
                * -(wanted * said.max(1e-7).ln() + (1.0 - wanted) * (1.0 - said).max(1e-7).ln())
                / cells;
            let at = grad.at(0, cy, cx);
            grad.data[at] = weight * (said - wanted) / cells;
            if present && said >= 0.5 {
                found += 1;
            } else if present {
                missed += 1;
            }
            let Some(target) = target else {
                continue;
            };
            // Where the box is, and which kind, only where there is something to say it of.
            let mut box_loss = 0.0;
            for (channel, wanted) in [
                (1, target.dx),
                (2, target.dy),
                (3, target.width),
                (4, target.height),
            ] {
                let said = sigmoid(head.data[head.at(channel, cy, cx)]);
                let difference = said - wanted;
                box_loss += difference * difference;
                let at = grad.at(channel, cy, cx);
                // Through the sigmoid, so a box that is far off moves further.
                grad.data[at] = 2.0 * difference * said * (1.0 - said) / cells;
            }
            loss += box_loss / cells;
            let logits: Vec<f32> = (0..ControlKind::COUNT)
                .map(|kind| head.data[head.at(5 + kind, cy, cx)])
                .collect();
            let top = logits.iter().copied().fold(f32::MIN, f32::max);
            let total: f32 = logits.iter().map(|value| (value - top).exp()).sum();
            for (kind, logit) in logits.iter().enumerate() {
                let said = (logit - top).exp() / total;
                let wanted = if kind == target.kind { 1.0 } else { 0.0 };
                if wanted > 0.0 {
                    loss += -said.max(1e-7).ln() / cells;
                }
                let at = grad.at(5 + kind, cy, cx);
                grad.data[at] = (said - wanted) / cells;
            }
        }
    }
    (loss, grad, found, missed)
}

impl Detector {
    /// Trains on a corpus, holding some of it back to be judged on, and says how each
    /// epoch went. The same corpus and the same seed give the same model.
    pub fn train(
        samples: &[Sample],
        training: Training,
        mut watching: impl FnMut(Epoch),
    ) -> Result<Self, crate::ReflexError> {
        if samples.is_empty() {
            return Err(crate::ReflexError::Corpus);
        }
        let mut model = Self::new(training.seed);
        let mut random = Random::seeded(training.seed ^ 0xfeed);
        let (order, teaching) = training.split(samples.len());
        let held_back = samples.len() - teaching;

        let mut gradients: Vec<Gradients> = model.layers.iter().map(Gradients::like).collect();
        let mut velocity: Vec<Gradients> = model.layers.iter().map(Gradients::like).collect();
        for epoch in 0..training.epochs {
            // A pass over the teaching part, in a different order each time.
            let mut pass: Vec<usize> = order[..teaching].to_vec();
            for index in (1..pass.len()).rev() {
                let other = (random.next() % (index as u64 + 1)) as usize;
                pass.swap(index, other);
            }
            let rate = training.learning_rate
                * (1.0 - epoch as f32 / training.epochs.max(1) as f32).max(0.05);
            let mut loss = 0.0;
            let (mut found, mut missed) = (0_u32, 0_u32);
            for sample in pass.iter().map(|index| &samples[*index]) {
                gradients.iter_mut().for_each(Gradients::clear);
                let input = Self::input_of(sample.tile.pixels());
                let activations = model.run(&input);
                let targets = targets_of(&sample.controls);
                let (sample_loss, mut grad, sample_found, sample_missed) =
                    head_gradient(activations.last().expect("a head"), &targets, &training);
                loss += sample_loss;
                found += sample_found;
                missed += sample_missed;
                for (index, layer) in model.layers.iter().enumerate().rev() {
                    let input = &activations[index];
                    grad = layer.backward(input, &grad, &mut gradients[index]);
                    // Back through the rectifier, which passes as little backwards as it
                    // passed forwards.
                    if index > 0 {
                        for (value, activation) in
                            grad.data.iter_mut().zip(activations[index].data.iter())
                        {
                            if *activation < 0.0 {
                                *value *= LEAK;
                            }
                        }
                    }
                }
                for (index, layer) in model.layers.iter_mut().enumerate() {
                    for (weight, (speed, gradient)) in layer.weights.iter_mut().zip(
                        velocity[index]
                            .weights
                            .iter_mut()
                            .zip(gradients[index].weights.iter()),
                    ) {
                        *speed = training.momentum * *speed - rate * gradient;
                        *weight += *speed;
                    }
                    for (bias, (speed, gradient)) in layer.biases.iter_mut().zip(
                        velocity[index]
                            .biases
                            .iter_mut()
                            .zip(gradients[index].biases.iter()),
                    ) {
                        *speed = training.momentum * *speed - rate * gradient;
                        *bias += *speed;
                    }
                }
            }

            let mut held_back_loss = 0.0;
            // What it says on the part it never trained on, and whether it is still a
            // model at all: how much its answer varies, and how much of it still fires.
            let mut said: Vec<f32> = Vec::new();
            let hidden: usize = model.layers[..model.layers.len() - 1]
                .iter()
                .map(|layer| layer.out_channels)
                .sum();
            let mut alive = vec![false; hidden];
            for sample in order[teaching..].iter().map(|index| &samples[*index]) {
                let activations = model.run(&Self::input_of(sample.tile.pixels()));
                let head = activations.last().expect("a head");
                for cy in 0..GRID_HEIGHT {
                    for cx in 0..GRID_WIDTH {
                        said.push(sigmoid(head.data[head.at(0, cy, cx)]));
                    }
                }
                let mut seen = 0;
                for hidden in &activations[1..activations.len() - 1] {
                    for channel in 0..hidden.channels {
                        let from = channel * hidden.height * hidden.width;
                        let plane = &hidden.data[from..from + hidden.height * hidden.width];
                        alive[seen + channel] |= plane.iter().any(|value| *value > 0.0);
                    }
                    seen += hidden.channels;
                }
                let (sample_loss, _, _, _) =
                    head_gradient(head, &targets_of(&sample.controls), &training);
                held_back_loss += sample_loss;
            }
            let mean = said.iter().sum::<f32>() / said.len().max(1) as f32;
            let spread = (said.iter().map(|value| (value - mean).powi(2)).sum::<f32>()
                / said.len().max(1) as f32)
                .sqrt();
            let seen = (found + missed).max(1) as f32;
            watching(Epoch {
                epoch,
                loss: loss / teaching.max(1) as f32,
                held_back_loss: held_back_loss / held_back.max(1) as f32,
                found_permille: ((found as f32 / seen) * 1_000.0) as u16,
                wrong_permille: ((missed as f32 / seen) * 1_000.0) as u16,
                spread_permille: (spread * 1_000.0).clamp(0.0, 1_000.0) as u16,
                alive_permille: ((alive.iter().filter(|on| **on).count() as f32
                    / alive.len().max(1) as f32)
                    * 1_000.0) as u16,
            });
        }
        Ok(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tile;

    /// A tile with one dark rectangle on a light ground, and the box that says where it is.
    fn window(x: usize, y: usize, width: usize, height: usize) -> Sample {
        let mut pixels = vec![235_u8; TILE_WIDTH * TILE_HEIGHT];
        for row in y..(y + height).min(TILE_HEIGHT) {
            for column in x..(x + width).min(TILE_WIDTH) {
                let edge =
                    row == y || column == x || row + 1 == y + height || column + 1 == x + width;
                pixels[row * TILE_WIDTH + column] = if edge { 40 } else { 150 };
            }
        }
        Sample {
            tile: Tile::from_pixels(pixels, 1_600, 960).unwrap(),
            controls: vec![Box2 {
                kind: ControlKind::Button,
                x: x as f32,
                y: y as f32,
                width: width as f32,
                height: height as f32,
            }],
        }
    }

    #[test]
    fn it_learns_where_a_control_is() {
        let mut samples = Vec::new();
        let mut random = Random::seeded(7);
        for _ in 0..12 {
            let x = 8 + (random.next() % 100) as usize;
            let y = 8 + (random.next() % 60) as usize;
            samples.push(window(x, y, 26, 12));
        }
        let mut last = None;
        let model = Detector::train(
            &samples,
            Training {
                epochs: 30,
                held_back: 0.25,
                // A dozen windows is a few hundred steps, so they are larger ones than a
                // real corpus needs. Leaking rectifiers are what makes that safe.
                learning_rate: 0.01,
                ..Training::default()
            },
            |epoch| last = Some(epoch),
        )
        .unwrap();
        let epoch = last.expect("an epoch");
        assert!(
            epoch.found_permille >= 800,
            "it should find nearly every control it was taught: {epoch:?}"
        );
        // The first run of this trainer learned for one epoch and then pushed most of its
        // own rectifiers under zero, after which it said the same thing about every cell
        // and its loss looked respectable. Both of these say that has not happened.
        assert!(
            epoch.alive_permille >= 700,
            "most of the model should still fire: {epoch:?}"
        );
        assert!(
            epoch.spread_permille >= 50,
            "it should not say the same thing everywhere: {epoch:?}"
        );

        // And on a window it never saw, the box it draws should be near the real one.
        let unseen = window(70, 40, 26, 12);
        let found = model.detect(unseen.tile.pixels(), 500);
        assert!(!found.is_empty(), "nothing found in a window like the rest");
        let (cx, cy) = found[0].control.centre();
        let (wx, wy) = unseen.controls[0].centre();
        assert!(
            (cx - wx).abs() <= 10.0 && (cy - wy).abs() <= 10.0,
            "found at ({cx}, {cy}) for a control at ({wx}, {wy})"
        );
        assert_eq!(found[0].control.kind, ControlKind::Button);
    }
}
