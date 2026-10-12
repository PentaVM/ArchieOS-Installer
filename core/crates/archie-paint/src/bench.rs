//! DRAW THE MONA LISA: a benchmark in which a painter sees a picture and may answer only
//! with the keyboard and the mouse, within fifteen seconds.
//!
//! The painter writes input events — the pen down, along and up, and keys that choose the
//! colour and the brush — one JSON object a line. They are replayed on a sheet of white
//! paper at the rate a real input device sends them ([`EVENTS_PER_SECOND`]); what did not
//! fit in the fifteen seconds, after the painter's own time to look and plan, is not drawn.
//! What was drawn is then compared with the picture: structurally ([`ssim`]), by colour
//! ([`colour_error`]), and by a critic model on Nebius, which the binary asks.
//!
//! The keyboard is the one paint applications share: `c`, six hex digits and Enter pick a
//! colour; `b`, the brush's diameter in pixels and Enter pick a brush.
//!
//! ```text
//! {"key":"c"} {"key":"8"} {"key":"a"} … {"key":"Enter"}      colour #8a…
//! {"key":"b"} {"key":"6"} {"key":"Enter"}                      a 6-pixel brush
//! {"mouse":"down","x":10,"y":12} {"mouse":"move","x":90,"y":12} {"mouse":"up"}
//! ```

use serde::{Deserialize, Serialize};

/// How fast input arrives at an application: a thousand events a second, the rate uinput
/// and xdotool sustain without dropping any.
pub const EVENTS_PER_SECOND: f32 = 1000.0;
/// The whole of the painter's time: looking, planning and every event.
pub const SECONDS: f32 = 15.0;

/// One thing the painter does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Input {
    Key {
        key: String,
    },
    Mouse {
        mouse: MouseAction,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseAction {
    Down,
    Move,
    Up,
}

impl Input {
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key { key: key.into() }
    }
    pub fn down(x: f32, y: f32) -> Self {
        Self::Mouse {
            mouse: MouseAction::Down,
            x,
            y,
        }
    }
    pub fn to(x: f32, y: f32) -> Self {
        Self::Mouse {
            mouse: MouseAction::Move,
            x,
            y,
        }
    }
    pub fn up() -> Self {
        Self::Mouse {
            mouse: MouseAction::Up,
            x: 0.0,
            y: 0.0,
        }
    }
}

/// The keys that choose a colour.
pub fn colour_keys(colour: [u8; 3]) -> Vec<Input> {
    let hex = format!("{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2]);
    std::iter::once(Input::key("c"))
        .chain(hex.chars().map(|c| Input::key(c.to_string())))
        .chain(std::iter::once(Input::key("Enter")))
        .collect()
}

/// The keys that choose a brush of that diameter.
pub fn brush_keys(diameter: u32) -> Vec<Input> {
    std::iter::once(Input::key("b"))
        .chain(
            diameter
                .to_string()
                .chars()
                .map(|c| Input::key(c.to_string())),
        )
        .chain(std::iter::once(Input::key("Enter")))
        .collect()
}

/// A sheet of paper in RGB.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 3]>,
}

impl Sheet {
    pub fn white(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![[255, 255, 255]; (width * height) as usize],
        }
    }

    fn put(&mut self, x: i32, y: i32, colour: [u8; 3], cover: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || cover <= 0.0 {
            return;
        }
        let at = (y as u32 * self.width + x as u32) as usize;
        let old = self.pixels[at];
        let cover = cover.min(1.0);
        for c in 0..3 {
            self.pixels[at][c] =
                (old[c] as f32 * (1.0 - cover) + colour[c] as f32 * cover).round() as u8;
        }
    }

    /// A round brush dragged from `a` to `b`: every pixel within the radius of the segment
    /// is covered, with a half-pixel soft edge.
    pub fn stroke(&mut self, a: (f32, f32), b: (f32, f32), diameter: f32, colour: [u8; 3]) {
        let radius = (diameter / 2.0).max(0.5);
        let (x0, x1) = (a.0.min(b.0) - radius - 1.0, a.0.max(b.0) + radius + 1.0);
        let (y0, y1) = (a.1.min(b.1) - radius - 1.0, a.1.max(b.1) + radius + 1.0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length2 = dx * dx + dy * dy;
        for y in (y0.floor() as i32)..=(y1.ceil() as i32) {
            for x in (x0.floor() as i32)..=(x1.ceil() as i32) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = if length2 > 0.0 {
                    (((px - a.0) * dx + (py - a.1) * dy) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
                let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                self.put(x, y, colour, radius + 0.5 - distance);
            }
        }
    }

    pub fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            let flat: Vec<u8> = self.pixels.iter().flatten().copied().collect();
            writer.write_image_data(&flat).expect("png data");
        }
        out
    }
}

/// What replaying the events came to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Replayed {
    pub events_given: usize,
    pub events_drawn: usize,
    pub strokes: usize,
    pub colour_changes: usize,
    pub seconds_of_input: f32,
}

/// Replays at most `budget` events on white paper. A key sequence left unfinished, or a
/// colour that is not six hex digits, changes nothing — as in an application.
pub fn replay(events: &[Input], width: u32, height: u32, budget: usize) -> (Sheet, Replayed) {
    let mut sheet = Sheet::white(width, height);
    let mut colour = [0u8, 0, 0];
    let mut diameter = 4.0f32;
    let mut typing: Option<(char, String)> = None;
    let mut pen: Option<(f32, f32)> = None;
    let (mut strokes, mut changes) = (0usize, 0usize);
    let drawn = events.len().min(budget);
    for event in &events[..drawn] {
        match event {
            Input::Key { key } => {
                let key = key.as_str();
                match (&mut typing, key) {
                    (None, "c") => typing = Some(('c', String::new())),
                    (None, "b") => typing = Some(('b', String::new())),
                    (Some((kind, text)), "Enter") => {
                        if *kind == 'c' && text.len() == 6 {
                            if let Ok(value) = u32::from_str_radix(text, 16) {
                                colour = [(value >> 16) as u8, (value >> 8) as u8, value as u8];
                                changes += 1;
                            }
                        } else if *kind == 'b' {
                            if let Ok(value) = text.parse::<f32>() {
                                diameter = value.clamp(1.0, 200.0);
                            }
                        }
                        typing = None;
                    }
                    (Some((_, text)), k) if k.chars().count() == 1 => text.push_str(k),
                    _ => typing = None,
                }
            }
            Input::Mouse { mouse, x, y } => match mouse {
                MouseAction::Down => {
                    pen = Some((*x, *y));
                    sheet.stroke((*x, *y), (*x, *y), diameter, colour);
                    strokes += 1;
                }
                MouseAction::Move => {
                    if let Some(from) = pen {
                        sheet.stroke(from, (*x, *y), diameter, colour);
                        pen = Some((*x, *y));
                    }
                }
                MouseAction::Up => pen = None,
            },
        }
    }
    let report = Replayed {
        events_given: events.len(),
        events_drawn: drawn,
        strokes,
        colour_changes: changes,
        seconds_of_input: drawn as f32 / EVENTS_PER_SECOND,
    };
    (sheet, report)
}

/// The picture, resampled (box filter) to the sheet's size.
pub fn fitted(picture: &crate::Picture, width: u32, height: u32) -> Sheet {
    let (pw, ph) = (picture.width() as f32, picture.height() as f32);
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let (sx0, sx1) = (
                x as f32 * pw / width as f32,
                (x + 1) as f32 * pw / width as f32,
            );
            let (sy0, sy1) = (
                y as f32 * ph / height as f32,
                (y + 1) as f32 * ph / height as f32,
            );
            let (mut sum, mut n) = ([0f32; 3], 0f32);
            for sy in (sy0 as u32)..(sy1.ceil() as u32).max(sy0 as u32 + 1) {
                for sx in (sx0 as u32)..(sx1.ceil() as u32).max(sx0 as u32 + 1) {
                    if let Some(c) =
                        picture.colour(sx.min(picture.width() - 1), sy.min(picture.height() - 1))
                    {
                        for k in 0..3 {
                            sum[k] += c[k] as f32;
                        }
                        n += 1.0;
                    }
                }
            }
            pixels.push([
                (sum[0] / n.max(1.0)) as u8,
                (sum[1] / n.max(1.0)) as u8,
                (sum[2] / n.max(1.0)) as u8,
            ]);
        }
    }
    Sheet {
        width,
        height,
        pixels,
    }
}

fn luma(c: [u8; 3]) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}

/// Structural similarity of the two sheets' light, over 8×8 windows (1 is identical).
pub fn ssim(a: &Sheet, b: &Sheet) -> f32 {
    let (c1, c2) = ((0.01f32 * 255.0).powi(2), (0.03f32 * 255.0).powi(2));
    let (mut total, mut windows) = (0.0f32, 0usize);
    for wy in (0..a.height.saturating_sub(7)).step_by(4) {
        for wx in (0..a.width.saturating_sub(7)).step_by(4) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for y in wy..wy + 8 {
                for x in wx..wx + 8 {
                    let at = (y * a.width + x) as usize;
                    let (p, q) = (luma(a.pixels[at]), luma(b.pixels[at]));
                    sa += p;
                    sb += q;
                    saa += p * p;
                    sbb += q * q;
                    sab += p * q;
                }
            }
            let n = 64.0;
            let (ma, mb) = (sa / n, sb / n);
            let (va, vb, cov) = (saa / n - ma * ma, sbb / n - mb * mb, sab / n - ma * mb);
            total += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            windows += 1;
        }
    }
    total / windows.max(1) as f32
}

fn lab(c: [u8; 3]) -> [f32; 3] {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    [
        116.0 * f(y) - 16.0,
        500.0 * (f(x) - f(y)),
        200.0 * (f(y) - f(z)),
    ]
}

/// Mean colour difference (CIE76 ΔE) over 4×4 blocks: how far the colours are, forgiving
/// the texture of strokes the eye blends at a distance.
pub fn colour_error(a: &Sheet, b: &Sheet) -> f32 {
    let (mut total, mut blocks) = (0f32, 0usize);
    for by in (0..a.height.saturating_sub(3)).step_by(4) {
        for bx in (0..a.width.saturating_sub(3)).step_by(4) {
            let (mut ma, mut mb) = ([0f32; 3], [0f32; 3]);
            for y in by..by + 4 {
                for x in bx..bx + 4 {
                    let at = (y * a.width + x) as usize;
                    for k in 0..3 {
                        ma[k] += a.pixels[at][k] as f32 / 16.0;
                        mb[k] += b.pixels[at][k] as f32 / 16.0;
                    }
                }
            }
            let (la, lb) = (
                lab([ma[0] as u8, ma[1] as u8, ma[2] as u8]),
                lab([mb[0] as u8, mb[1] as u8, mb[2] as u8]),
            );
            total += ((la[0] - lb[0]).powi(2) + (la[1] - lb[1]).powi(2) + (la[2] - lb[2]).powi(2))
                .sqrt();
            blocks += 1;
        }
    }
    total / blocks.max(1) as f32
}

/// A region the painter should spend its detail on, in sheet pixels, with a weight.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Focus {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub weight: f32,
}

/// The realist: a deterministic painter that works coarse to fine. A broad brush lays
/// every part of the picture in its colour, row by row; then finer brushes go only where
/// the paper is still furthest from the picture — more so inside `focus` — until the event
/// budget is spent. Strokes are grouped by brush and colour, so the keyboard is used once
/// per colour rather than once per stroke.
pub fn realist(target: &Sheet, focus: &[Focus], budget: usize) -> Vec<Input> {
    realist_with(target, focus, budget, &Realist::default())
}

/// How the realist paints: its palette, its brushes coarse to fine with the share of the
/// budget each may take, and how much more a stroke inside IRIS's focus is worth.
#[derive(Debug, Clone, PartialEq)]
pub struct Realist {
    pub palette: usize,
    pub layers: Vec<(u32, f32)>,
    pub focus_strength: f32,
}

impl Default for Realist {
    /// Tuned on the Mona Lisa (384×572, 15 s): a finer first brush ran out of events and left
    /// paper showing (ΔE 27–32); a 24-pixel one covers everything and leaves most of the budget
    /// for detail (SSIM 0.63, ΔE 5.2, against 0.46 and 12.1 for 12 px and 32 colours).
    fn default() -> Self {
        Self {
            palette: 24,
            layers: vec![(24, 1.0), (12, 0.3), (6, 0.3), (3, 0.25), (2, 0.15)],
            focus_strength: 0.3,
        }
    }
}

/// [`realist`] with its settings given.
pub fn realist_with(
    target: &Sheet,
    focus: &[Focus],
    budget: usize,
    settings: &Realist,
) -> Vec<Input> {
    let palette = palette_of(target, settings.palette.max(2));
    let nearest = |c: [u8; 3]| -> usize {
        (0..palette.len())
            .min_by_key(|&i| {
                let p = palette[i];
                (0..3)
                    .map(|k| (p[k] as i32 - c[k] as i32).pow(2))
                    .sum::<i32>()
            })
            .unwrap_or(0)
    };
    let weight_at = |x: f32, y: f32| -> f32 {
        focus
            .iter()
            .filter(|f| x >= f.x0 && x <= f.x1 && y >= f.y0 && y <= f.y1)
            .fold(1.0f32, |w, f| {
                w.max(1.0 + (f.weight - 1.0) * settings.focus_strength)
            })
    };
    let mut events = Vec::new();
    let mut sheet = Sheet::white(target.width, target.height);
    // Layer by layer, coarse to fine. The first layer covers everything; later layers
    // compete for what is left of the budget by how much each stroke would help.
    let layers = &settings.layers;
    for (layer, &(diameter, share)) in layers.iter().enumerate() {
        let remaining = budget.saturating_sub(events.len());
        if remaining < 16 {
            break;
        }
        let allowance = if layer == 0 {
            remaining
        } else {
            ((remaining as f32) * share / layers[layer..].iter().map(|l| l.1).sum::<f32>())
                .max(16.0) as usize
        };
        // Runs of one palette colour along each row of this brush's height.
        let step = diameter.max(1);
        let mut candidates: Vec<(usize, u32, u32, u32, f32)> = Vec::new(); // colour, y, x0, x1, gain
        let mut y = step / 2;
        while y < target.height {
            let mut x = 0u32;
            while x < target.width {
                let colour = nearest(block_mean(target, x, y, step));
                let start = x;
                let mut gain = 0.0f32;
                while x < target.width && nearest(block_mean(target, x, y, step)) == colour {
                    let want = target.pixels[(y * target.width + x) as usize];
                    let have = sheet.pixels[(y * target.width + x) as usize];
                    let before: f32 = (0..3)
                        .map(|k| (want[k] as f32 - have[k] as f32).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    let after: f32 = (0..3)
                        .map(|k| (want[k] as f32 - palette[colour][k] as f32).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    gain += (before - after).max(0.0) * weight_at(x as f32, y as f32) * step as f32;
                    x += 1;
                }
                if layer == 0 || gain > 0.0 {
                    candidates.push((colour, y, start, x - 1, gain));
                }
            }
            y += step;
        }
        if layer > 0 {
            candidates.sort_by(|a, b| b.4.total_cmp(&a.4));
        }
        // Take strokes (three events each) until the allowance, counting the keys too.
        let mut chosen: Vec<(usize, u32, u32, u32)> = Vec::new();
        let mut cost = brush_keys(diameter).len();
        let mut colours_used = std::collections::BTreeSet::new();
        for (colour, y, x0, x1, _) in candidates {
            let extra = 3 + if colours_used.contains(&colour) { 0 } else { 8 };
            if cost + extra > allowance {
                if layer == 0 {
                    continue;
                }
                break;
            }
            cost += extra;
            colours_used.insert(colour);
            chosen.push((colour, y, x0, x1));
        }
        chosen.sort_by_key(|&(colour, y, x0, _)| (colour, y, x0));
        events.extend(brush_keys(diameter));
        let mut current = usize::MAX;
        for (colour, y, x0, x1) in chosen {
            if colour != current {
                events.extend(colour_keys(palette[colour]));
                current = colour;
            }
            let (ya, xa, xb) = (y as f32 + 0.5, x0 as f32 + 0.5, x1 as f32 + 0.5);
            events.push(Input::down(xa, ya));
            events.push(Input::to(xb, ya));
            events.push(Input::up());
            sheet.stroke((xa, ya), (xb, ya), diameter as f32, palette[colour]);
        }
    }
    events.truncate(budget);
    events
}

fn block_mean(sheet: &Sheet, x: u32, y: u32, size: u32) -> [u8; 3] {
    let half = size / 2;
    let (y0, y1) = (y.saturating_sub(half), (y + half + 1).min(sheet.height));
    let (mut sum, mut n) = ([0u32; 3], 0u32);
    for yy in y0..y1 {
        let at = (yy * sheet.width + x) as usize;
        for k in 0..3 {
            sum[k] += sheet.pixels[at][k] as u32;
        }
        n += 1;
    }
    [
        (sum[0] / n.max(1)) as u8,
        (sum[1] / n.max(1)) as u8,
        (sum[2] / n.max(1)) as u8,
    ]
}

/// A palette of `k` colours by k-means over the picture, seeded deterministically from its
/// light, so the same picture always gets the same palette.
pub fn palette_of(sheet: &Sheet, k: usize) -> Vec<[u8; 3]> {
    let samples: Vec<[f32; 3]> = sheet
        .pixels
        .iter()
        .step_by(7)
        .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
        .collect();
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| (a[0] + a[1] + a[2]).total_cmp(&(b[0] + b[1] + b[2])));
    let mut centres: Vec<[f32; 3]> = (0..k)
        .map(|i| sorted[(i * sorted.len()) / k + sorted.len() / (2 * k)])
        .collect();
    for _ in 0..12 {
        let mut sums = vec![[0f32; 3]; k];
        let mut counts = vec![0usize; k];
        for s in &samples {
            let best = (0..k)
                .min_by(|&a, &b| distance(&centres[a], s).total_cmp(&distance(&centres[b], s)))
                .unwrap_or(0);
            for c in 0..3 {
                sums[best][c] += s[c];
            }
            counts[best] += 1;
        }
        for i in 0..k {
            if counts[i] > 0 {
                centres[i] = [
                    sums[i][0] / counts[i] as f32,
                    sums[i][1] / counts[i] as f32,
                    sums[i][2] / counts[i] as f32,
                ];
            }
        }
    }
    centres
        .iter()
        .map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8])
        .collect()
}

fn distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_choose_colour_and_brush_and_the_pen_draws() {
        let mut events = brush_keys(4);
        events.extend(colour_keys([255, 0, 0]));
        events.extend([Input::down(2.0, 5.0), Input::to(18.0, 5.0), Input::up()]);
        let (sheet, report) = replay(&events, 20, 10, usize::MAX);
        assert_eq!(sheet.pixels[(5 * 20 + 10) as usize], [255, 0, 0]);
        assert_eq!(
            sheet.pixels[(0 * 20 + 10) as usize],
            [255, 255, 255],
            "the paper above the line is untouched"
        );
        assert_eq!((report.strokes, report.colour_changes), (1, 1));
    }

    #[test]
    fn what_does_not_fit_in_the_budget_is_not_drawn() {
        let mut events = colour_keys([0, 0, 0]);
        events.extend([Input::down(2.0, 5.0), Input::to(18.0, 5.0), Input::up()]);
        let (sheet, report) = replay(&events, 20, 10, 5);
        assert_eq!(report.events_drawn, 5);
        assert!(
            sheet.pixels.iter().all(|p| *p == [255, 255, 255]),
            "the colour was never finished"
        );
    }

    #[test]
    fn events_are_json_lines_as_the_protocol_says() {
        let line = serde_json::to_string(&Input::to(1.5, 2.0)).unwrap();
        assert_eq!(line, r#"{"mouse":"move","x":1.5,"y":2.0}"#);
        let back: Input = serde_json::from_str(r#"{"key":"Enter"}"#).unwrap();
        assert_eq!(back, Input::key("Enter"));
        let down: Input = serde_json::from_str(r#"{"mouse":"up"}"#).unwrap();
        assert_eq!(down, Input::up());
    }

    #[test]
    fn identical_sheets_score_perfectly_and_white_paper_does_not() {
        let mut picture = Sheet::white(64, 64);
        for (i, p) in picture.pixels.iter_mut().enumerate() {
            *p = [(i % 64 * 4) as u8, (i / 64 * 4) as u8, 90];
        }
        assert!((ssim(&picture, &picture) - 1.0).abs() < 1e-4);
        assert!(colour_error(&picture, &picture) < 1e-3);
        let blank = Sheet::white(64, 64);
        assert!(colour_error(&picture, &blank) > 20.0);
    }

    #[test]
    fn the_realist_paints_a_picture_within_its_budget() {
        let mut picture = Sheet::white(96, 128);
        for y in 0..128u32 {
            for x in 0..96u32 {
                let inside = (x as i32 - 48).pow(2) + (y as i32 - 60).pow(2) < 900;
                picture.pixels[(y * 96 + x) as usize] = if inside {
                    [200, 150, 110]
                } else {
                    [40, 60, 50]
                };
            }
        }
        let events = realist(&picture, &[], 3000);
        assert!(events.len() <= 3000);
        let (drawn, _) = replay(&events, 96, 128, usize::MAX);
        assert!(
            colour_error(&picture, &drawn) < 8.0,
            "{}",
            colour_error(&picture, &drawn)
        );
        assert!(ssim(&picture, &drawn) > 0.6, "{}", ssim(&picture, &drawn));
    }
}
