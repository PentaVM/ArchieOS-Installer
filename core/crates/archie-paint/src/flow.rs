//! Lines that follow the form.
//!
//! Hatching is fast to plan and honest about tone, and it looks like hatching: a grid laid
//! over a face. A hand does something else â it draws *along* what it sees, so the line
//! itself says where the cheek turns. This plans that: the direction at each point where
//! the picture changes least, a line let go into that field, and tone from how many lines
//! are let go rather than from how close a grid is.
//!
//! It is arithmetic, not a model: a gradient, an average over a neighbourhood, and paths
//! integrated through it. Nothing here recognises anything.

use crate::{Canvas, Picture, Plan};
use archie_stage::StrokePoint;

/// How to draw with lines that follow the form.
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    /// How far apart the seeds of the lines are, in canvas pixels. Fewer, further apart,
    /// is a sparser drawing and a faster one.
    pub seeding: u32,
    /// How long a step along the field is. Short steps follow the form closely and cost
    /// points; long ones are cheaper and straighter.
    pub step: f32,
    /// The most steps one line may take before the pen lifts.
    pub steps: usize,
    /// How dark a place has to be for a line to start there or carry on through it.
    pub darker_than: u8,
    /// How far over the picture the direction field is averaged. A wide average gives calm,
    /// long lines; a narrow one follows every wrinkle.
    pub coherence: u32,
    /// How much the picture has to change across a place for a line to keep going through
    /// it, as a rank among the picture's own places: 0.0 lets a line run anywhere, 0.4 keeps
    /// it to the more shaped three fifths, 0.9 to the ten sharpest hundredths. Flat ground
    /// has no form to follow, and a line that carries on through it is ruled at random.
    pub least_form: f32,
    /// How many lines to keep at most, the longest first, or none for all of them. A line
    /// costs the same to send whether it is long or short, so when there is only time for a
    /// few, the few should be the ones that carry the most of the picture.
    pub most: usize,
    /// How long the drawing may take, in seconds, or none for no limit.
    ///
    /// This is the only budget a drawing really has, and stating it in seconds rather than
    /// in lines is what makes the plan honest. An application looks at the pointer at its
    /// own rate and joins what it saw with straight runs, so there is a speed past which a
    /// line arrives as a polygon; that fixes how much line a second can carry. Everything
    /// else â how many commands, how far the pointer travels between them â only decides
    /// how much of each second goes into line rather than into getting there.
    ///
    /// Given a time, the lines are chosen the way a hand chooses them: at each turn, the one
    /// that says the most about the picture for the time it costs to go and draw it.
    pub seconds: f32,
    /// How fast the pointer will be drawing, in pixels a second.
    pub speed: f32,
    /// What one command costs before any of it is drawing: the round trip, and whatever the
    /// stage draws of its own. Measured on this stage at video tempo, with the same drawing
    /// sent one line at a time and then in batches: eighty-four milliseconds.
    pub per_command: f32,
    /// What one line costs beyond drawing it: the pen going down and coming up, and the
    /// travel to it taken at the tempo's pace rather than at the drawing speed. Measured on
    /// this stage at video tempo by planning a drawing and timing it: forty-three
    /// milliseconds a line.
    pub per_line: f32,
    /// What putting one colour down and picking another up costs: the command, and the
    /// pointer going to wherever the application keeps its colours and coming back. Measured
    /// at about three tenths of a second, and it is the reason a hand does not change brush
    /// every other stroke.
    pub per_colour: f32,
    /// How many times the pen may go over the darkest lines.
    ///
    /// Every line the same weight is what makes a drawing look ruled rather than drawn. A
    /// hand presses harder where the picture is dark, and a pointer that cannot press can do
    /// the same thing by going over a line again, a hair to one side. It costs ink and no
    /// commands, so the budget pays for it out of the same purse as any other line.
    pub weight: u32,
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            seeding: 7,
            step: 4.0,
            steps: 120,
            darker_than: 96,
            coherence: 6,
            least_form: 0.4,
            most: 0,
            seconds: 0.0,
            speed: 950.0,
            per_command: 0.084,
            per_line: 0.043,
            per_colour: 0.30,
            weight: 1,
        }
    }
}

/// The direction at each point in which the picture changes least, and how strongly it
/// changes across that direction.
struct Field {
    along: Vec<(f32, f32)>,
    /// Where that place ranks among the picture's own places for how strongly it changes:
    /// 0.0 is the flattest ground in this picture, 1.0 the sharpest. A rank rather than a
    /// measure, because one hard edge should not decide that the rest of a soft painting
    /// is featureless.
    form: Vec<f32>,
    width: usize,
    height: usize,
}

impl Field {
    fn at(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let at = self.index(x, y)?;
        Some(self.along[at])
    }

    fn form_at(&self, x: f32, y: f32) -> Option<f32> {
        let at = self.index(x, y)?;
        Some(self.form[at])
    }

    fn index(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (ix, iy) = (x as usize, y as usize);
        if ix >= self.width || iy >= self.height {
            return None;
        }
        Some(iy * self.width + ix)
    }
}

/// Every value replaced by its rank among the others, from 0.0 for the smallest to 1.0 for
/// the largest. Counted in bins rather than sorted: the picture has a million places and
/// their exact order does not matter, only where each one stands among the rest.
fn rank(values: &mut [f32]) {
    const BINS: usize = 4096;
    let largest = values.iter().copied().fold(0.0_f32, f32::max);
    if largest <= 0.0 {
        values.fill(0.0);
        return;
    }
    let bin_of = |value: f32| ((value / largest) * (BINS - 1) as f32) as usize;
    let mut counted = vec![0_u32; BINS];
    for value in values.iter() {
        counted[bin_of(*value)] += 1;
    }
    let mut running = 0_u32;
    let mut below = vec![0.0_f32; BINS];
    let total = values.len().max(1) as f32;
    for (bin, count) in counted.iter().enumerate() {
        below[bin] = f64::from(running) as f32 / total;
        running += count;
    }
    for value in values.iter_mut() {
        *value = below[bin_of(*value)];
    }
}

/// Reads the picture once: the gradient at every pixel, turned a quarter and averaged over
/// a neighbourhood so that neighbouring points agree about which way the form runs.
fn field_of(picture: &Picture, coherence: u32) -> Field {
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    let light = |x: usize, y: usize| f32::from(picture.grey(x as u32, y as u32).unwrap_or(255));
    // The structure tensor, which averages directions without cancelling opposite ones.
    let mut xx = vec![0.0_f32; width * height];
    let mut yy = vec![0.0_f32; width * height];
    let mut xy = vec![0.0_f32; width * height];
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let gx = (light(x + 1, y - 1) + 2.0 * light(x + 1, y) + light(x + 1, y + 1))
                - (light(x - 1, y - 1) + 2.0 * light(x - 1, y) + light(x - 1, y + 1));
            let gy = (light(x - 1, y + 1) + 2.0 * light(x, y + 1) + light(x + 1, y + 1))
                - (light(x - 1, y - 1) + 2.0 * light(x, y - 1) + light(x + 1, y - 1));
            let at = y * width + x;
            xx[at] = gx * gx;
            yy[at] = gy * gy;
            xy[at] = gx * gy;
        }
    }
    let span = coherence.max(1) as i64;
    let mut along = vec![(1.0_f32, 0.0_f32); width * height];
    let mut form = vec![0.0_f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
            for dy in -span..=span {
                for dx in -span..=span {
                    let nx = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                    let ny = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                    let at = ny * width + nx;
                    sxx += xx[at];
                    syy += yy[at];
                    sxy += xy[at];
                }
            }
            // The smaller eigenvector of that tensor is the way the picture changes least.
            let difference = sxx - syy;
            let root = (difference * difference + 4.0 * sxy * sxy).sqrt();
            let smaller = (sxx + syy - root) / 2.0;
            let larger = (sxx + syy + root) / 2.0;
            let (mut vx, mut vy) = (sxy, smaller - sxx);
            let length = (vx * vx + vy * vy).sqrt();
            if length < 1e-6 {
                vx = 1.0;
                vy = 0.0;
            } else {
                vx /= length;
                vy /= length;
            }
            along[y * width + x] = (vx, vy);
            form[y * width + x] = larger;
        }
    }
    // Ranked, so that the same setting reads a soft painting and a hard drawing alike.
    rank(&mut form);
    Field {
        along,
        form,
        width,
        height,
    }
}

/// The lines to draw, in the order to draw them: darkest first, so that a picture being
/// watched becomes recognisable early.
///
/// `palette` is what the application can be told to draw with â its swatches. Each line
/// takes the nearest one to the picture under it, and the plan comes back grouped by
/// colour, because changing colour costs a command and drawing does not.
pub fn plan(
    picture: &Picture,
    canvas: Canvas,
    flow: &Flow,
    palette: &[[u8; 3]],
    max_points: usize,
) -> Vec<(usize, Plan)> {
    let field = field_of(picture, flow.coherence);
    let fitted = crate::fit_into(picture, canvas);
    let seeding = flow.seeding.max(1);
    // Seeds where the picture is dark, darkest first: the shadow that makes a face a face
    // is drawn before the shading around it.
    let mut seeds: Vec<(u8, i32, i32)> = Vec::new();
    let mut y = fitted.y;
    while y < fitted.y + fitted.height as i32 {
        let mut x = fitted.x;
        while x < fitted.x + fitted.width as i32 {
            if let Some(dark) = fitted.darkness_at_canvas(picture, x as f32, y as f32) {
                if dark >= flow.darker_than {
                    seeds.push((dark, x, y));
                }
            }
            x += seeding as i32;
        }
        y += seeding as i32;
    }
    seeds.sort_by_key(|seed| std::cmp::Reverse(seed.0));

    let mut drawn: Vec<(usize, Vec<StrokePoint>)> = Vec::new();
    for (_, sx, sy) in seeds {
        let Some(stroke) = follow(picture, &fitted, &field, flow, sx, sy, max_points) else {
            continue;
        };
        let at = nearest(palette, fitted.colour_at(picture, sx as f32, sy as f32));
        drawn.push((at, stroke));
    }
    if flow.seconds > 0.0 {
        drawn = what_fits_in_the_time(&drawn, picture, &fitted, flow);
    } else if flow.most > 0 && drawn.len() > flow.most {
        // Kept by length, but drawn in the order they were found, so the picture still
        // arrives darkest first rather than longest first.
        let mut order: Vec<usize> = (0..drawn.len()).collect();
        order.sort_by(|a, b| {
            length_of(&drawn[*b].1)
                .partial_cmp(&length_of(&drawn[*a].1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        order.truncate(flow.most);
        let keeping: std::collections::HashSet<usize> = order.into_iter().collect();
        let mut at = 0;
        drawn.retain(|_| {
            at += 1;
            keeping.contains(&(at - 1))
        });
    }

    if flow.seconds > 0.0 {
        // The order is the choosing: the lines come out in the order a hand would make them,
        // and a run of them in one colour is one visit to the palette.
        let mut runs: Vec<(usize, Plan)> = Vec::new();
        for (at, stroke) in drawn {
            match runs.last_mut() {
                Some((colour, plan)) if *colour == at => plan.strokes.push(stroke),
                _ => runs.push((
                    at,
                    Plan {
                        strokes: vec![stroke],
                    },
                )),
            }
        }
        return runs;
    }

    let mut by_colour: Vec<Plan> = vec![Plan::default(); palette.len().max(1)];
    for (at, stroke) in drawn {
        by_colour[at].strokes.push(stroke);
    }
    // The pointer has to travel from the end of one line to the start of the next, and that
    // travel is time in which nothing is drawn. Ordering the lines so that each is near the
    // one before turns some of it back into drawing.
    for plan in &mut by_colour {
        shortest_way_round(&mut plan.strokes);
    }
    by_colour
        .into_iter()
        .enumerate()
        .filter(|(_, plan)| !plan.is_empty())
        .collect()
}

/// The lines that fit in the time there is, in the order a hand would make them.
///
/// Greedy, and the greed is the whole idea: at each turn it takes the line that darkens the
/// most picture nobody has darkened yet, per second it costs to go and draw it â and the
/// cost is the honest one, the travel to reach it, the drawing of it, the command it takes,
/// and one more command when it means putting down one colour and picking up another. A long
/// line across ground already covered is worth less than a short one through a face nobody
/// has drawn; a good line on the far side of the picture may be worth less than a fair one
/// under the pointer. That is the trade a hand makes without thinking about it.
///
/// The picture is tracked coarsely â a tile for every few pixels â because the question is
/// "has this part been said yet", not "is this pixel right".
fn what_fits_in_the_time(
    lines: &[(usize, Vec<StrokePoint>)],
    picture: &Picture,
    fitted: &crate::Fitted,
    flow: &Flow,
) -> Vec<(usize, Vec<StrokePoint>)> {
    const TILE: i32 = 6;
    let speed = if flow.speed > 0.0 { flow.speed } else { 950.0 };
    let across = (fitted.width as i32 / TILE + 2).max(1) as usize;
    let down = (fitted.height as i32 / TILE + 2).max(1) as usize;
    let tile_of = |point: &StrokePoint| -> Option<usize> {
        let (x, y) = (point.x - fitted.x, point.y - fitted.y);
        if x < 0 || y < 0 {
            return None;
        }
        let (tx, ty) = ((x / TILE) as usize, (y / TILE) as usize);
        (tx < across && ty < down).then_some(ty * across + tx)
    };
    // What each line would say, as the tiles it touches and how dark the picture is there.
    let says: Vec<Vec<(usize, f32)>> = lines
        .iter()
        .map(|(_, stroke)| {
            let mut tiles: Vec<(usize, f32)> = stroke
                .iter()
                .filter_map(|point| {
                    let tile = tile_of(point)?;
                    let dark = fitted
                        .darkness_at_canvas(picture, point.x as f32, point.y as f32)
                        .unwrap_or(0);
                    Some((tile, f32::from(dark) / 255.0))
                })
                .collect();
            tiles.sort_by_key(|(tile, _)| *tile);
            tiles.dedup_by_key(|(tile, _)| *tile);
            tiles
        })
        .collect();
    let lengths: Vec<f32> = lines.iter().map(|(_, stroke)| length_of(stroke)).collect();
    // How heavy each line should be: the darkest go over twice or three times, which is
    // what a hand does by pressing, and what a pointer that cannot press has instead.
    let weights: Vec<u32> = says
        .iter()
        .map(|tiles| {
            if tiles.is_empty() || flow.weight <= 1 {
                return 1;
            }
            let mean: f32 = tiles.iter().map(|(_, dark)| *dark).sum::<f32>() / tiles.len() as f32;
            // Nothing below halfway is worth a second pass; the darkest get them all.
            let over = ((mean - 0.5) / 0.5).clamp(0.0, 1.0);
            1 + (over * (flow.weight - 1) as f32).round() as u32
        })
        .collect();

    let mut said = vec![false; across * down];
    let mut taken = vec![false; lines.len()];
    let mut order: Vec<usize> = Vec::new();
    let mut spent = 0.0_f32;
    let mut pen: Option<StrokePoint> = None;
    let mut in_hand: Option<usize> = None;
    loop {
        let mut best: Option<(usize, f32, f32)> = None;
        for at in 0..lines.len() {
            if taken[at] {
                continue;
            }
            let adds: f32 = says[at]
                .iter()
                .map(|(tile, dark)| if said[*tile] { 0.0 } else { *dark })
                .sum();
            if adds <= 0.0 {
                continue;
            }
            let start = lines[at].1[0];
            let travel = match pen {
                Some(from) => {
                    let (dx, dy) = ((start.x - from.x) as f32, (start.y - from.y) as f32);
                    (dx * dx + dy * dy).sqrt()
                }
                None => 0.0,
            };
            // Lines travel together: a command carries as many as it may, so a line costs
            // its share of one. Changing colour costs a whole command, because putting the
            // colour down and picking another up is a click of its own.
            let share = flow.per_command / crate::MAX_STROKE_PATHS as f32;
            let palette = if in_hand == Some(lines[at].0) {
                0.0
            } else {
                flow.per_colour
            };
            let costs = (travel + lengths[at] * weights[at] as f32) / speed
                + weights[at] as f32 * flow.per_line
                + share
                + palette;
            if spent + costs > flow.seconds {
                continue;
            }
            let worth = adds / costs.max(1e-6);
            if best.is_none_or(|(_, so_far, _)| worth > so_far) {
                best = Some((at, worth, costs));
            }
        }
        let Some((at, _, costs)) = best else {
            break;
        };
        taken[at] = true;
        spent += costs;
        pen = lines[at].1.last().copied();
        in_hand = Some(lines[at].0);
        for (tile, _) in &says[at] {
            said[*tile] = true;
        }
        order.push(at);
    }
    order
        .into_iter()
        .flat_map(|at| {
            let (colour, line) = lines[at].clone();
            let mut passes = vec![(colour, line.clone())];
            // Each further pass a hair to one side of the last, so the mark widens instead
            // of landing on the same pixels twice — and drawn back the way it came, because
            // going round to the start again is travel that draws nothing.
            for pass in 1..weights[at] {
                let mut over = beside(&line, pass as i32);
                if pass % 2 == 1 {
                    over.reverse();
                }
                passes.push((colour, over));
            }
            passes
        })
        .collect()
}

/// The same line, moved a little to one side of itself.
fn beside(line: &[StrokePoint], by: i32) -> Vec<StrokePoint> {
    line.iter()
        .enumerate()
        .map(|(at, point)| {
            let next = line[(at + 1).min(line.len() - 1)];
            let previous = line[at.saturating_sub(1)];
            let (dx, dy) = ((next.x - previous.x) as f32, (next.y - previous.y) as f32);
            let length = (dx * dx + dy * dy).sqrt();
            if length < 1e-3 {
                return *point;
            }
            // Across the line, not along it.
            StrokePoint {
                x: point.x + (-dy / length * by as f32).round() as i32,
                y: point.y + (dx / length * by as f32).round() as i32,
            }
        })
        .collect()
}

/// The lines put in an order that walks between them rather than across the picture and
/// back. Nearest first, starting from the longest, which is also how a hand goes about it:
/// the large shape, then what is next to it.
fn shortest_way_round(lines: &mut Vec<Vec<StrokePoint>>) {
    if lines.len() < 3 {
        return;
    }
    let start = (0..lines.len())
        .max_by(|a, b| {
            length_of(&lines[*a])
                .partial_cmp(&length_of(&lines[*b]))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0);
    let mut left: Vec<Vec<StrokePoint>> = std::mem::take(lines);
    let mut order: Vec<Vec<StrokePoint>> = Vec::with_capacity(left.len());
    order.push(left.swap_remove(start));
    while !left.is_empty() {
        let from = *order
            .last()
            .and_then(|line| line.last())
            .unwrap_or(&StrokePoint { x: 0, y: 0 });
        let near = (0..left.len())
            .min_by_key(|at| {
                let to = left[*at][0];
                let (dx, dy) = (to.x - from.x, to.y - from.y);
                dx * dx + dy * dy
            })
            .unwrap_or(0);
        order.push(left.swap_remove(near));
    }
    *lines = order;
}

/// How far the pointer travels along a line.
pub fn length_of(stroke: &[StrokePoint]) -> f32 {
    stroke
        .windows(2)
        .map(|pair| {
            let (dx, dy) = (pair[1].x - pair[0].x, pair[1].y - pair[0].y);
            (((dx * dx + dy * dy) as f32).sqrt()).abs()
        })
        .sum()
}

/// One line let go into the field from that point, forwards and back, in canvas pixels.
fn follow(
    picture: &Picture,
    fitted: &crate::Fitted,
    field: &Field,
    flow: &Flow,
    sx: i32,
    sy: i32,
    max_points: usize,
) -> Option<Vec<StrokePoint>> {
    let mut points: Vec<(f32, f32)> = Vec::new();
    for backwards in [false, true] {
        let (mut x, mut y) = (sx as f32, sy as f32);
        let mut last: Option<(f32, f32)> = None;
        let mut taken = Vec::new();
        for _ in 0..flow.steps / 2 {
            let Some((px, py)) = fitted.in_picture(picture, x, y) else {
                break;
            };
            let Some((mut vx, mut vy)) = field.at(px, py) else {
                break;
            };
            // Where nothing changes there is no form to follow, and a line that carries on
            // is a line ruled at random across the picture.
            match field.form_at(px, py) {
                Some(form) if form >= flow.least_form => {}
                _ => break,
            }
            if backwards {
                vx = -vx;
                vy = -vy;
            }
            // A line does not double back on itself; where the field flips sign, follow it.
            if let Some((lx, ly)) = last {
                if lx * vx + ly * vy < 0.0 {
                    vx = -vx;
                    vy = -vy;
                }
            }
            last = Some((vx, vy));
            taken.push((x, y));
            x += vx * flow.step;
            y += vy * flow.step;
            match fitted.darkness_at_canvas(picture, x, y) {
                Some(dark) if dark >= flow.darker_than => {}
                _ => break,
            }
        }
        if backwards {
            taken.reverse();
            taken.extend(points.iter().copied());
            points = taken;
        } else {
            points = taken;
        }
    }
    if points.len() < 2 {
        return None;
    }
    // Smoothed before it is thinned: a line the application will only see a dozen points of
    // should be a line that a dozen points describe.
    let smoothed: Vec<(f32, f32)> = (0..points.len())
        .map(|at| {
            let from = at.saturating_sub(2);
            let to = (at + 3).min(points.len());
            let taken = &points[from..to];
            let count = taken.len() as f32;
            (
                taken.iter().map(|(x, _)| x).sum::<f32>() / count,
                taken.iter().map(|(_, y)| y).sum::<f32>() / count,
            )
        })
        .collect();
    let points = smoothed;
    let step = points.len().div_ceil(max_points.max(2));
    let mut stroke: Vec<StrokePoint> = points
        .iter()
        .step_by(step.max(1))
        .map(|(x, y)| StrokePoint {
            x: x.round() as i32,
            y: y.round() as i32,
        })
        .collect();
    if let Some(last) = points.last() {
        let end = StrokePoint {
            x: last.0.round() as i32,
            y: last.1.round() as i32,
        };
        if stroke.last() != Some(&end) {
            stroke.push(end);
        }
    }
    stroke.dedup();
    (stroke.len() >= 2).then_some(stroke)
}

/// The swatch nearest to a colour, or the first when the picture had no colour.
fn nearest(palette: &[[u8; 3]], colour: Option<[u8; 3]>) -> usize {
    let Some(colour) = colour else { return 0 };
    palette
        .iter()
        .enumerate()
        .min_by_key(|(_, swatch)| {
            swatch
                .iter()
                .zip(colour)
                .map(|(a, b)| {
                    let d = i32::from(*a) - i32::from(b);
                    d * d
                })
                .sum::<i32>()
        })
        .map(|(at, _)| at)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture of a dark band running across it: every line should run the same way.
    fn band(size: u32) -> Picture {
        let grey = (0..size * size)
            .map(|at| {
                let y = at / size;
                if y > size / 3 && y < size * 2 / 3 {
                    20
                } else {
                    250
                }
            })
            .collect();
        Picture::from_grey(grey, size, size).unwrap()
    }

    #[test]
    fn its_lines_run_along_the_form_not_across_it() {
        let picture = band(120);
        let canvas = Canvas::new(0, 0, 120, 120);
        let groups = plan(&picture, canvas, &Flow::default(), &[[0, 0, 0]], 64);
        let strokes: Vec<_> = groups.iter().flat_map(|(_, plan)| &plan.strokes).collect();
        assert!(!strokes.is_empty(), "a dark band is something to draw");
        let mut along = 0;
        for stroke in &strokes {
            let (first, last) = (stroke[0], stroke[stroke.len() - 1]);
            if (last.x - first.x).abs() > (last.y - first.y).abs() {
                along += 1;
            }
        }
        assert!(
            along * 4 >= strokes.len() * 3,
            "three lines in four should run along the band: {along} of {}",
            strokes.len()
        );
    }

    #[test]
    fn it_leaves_the_light_alone_and_groups_by_colour() {
        let white = Picture::from_grey(vec![255; 80 * 80], 80, 80).unwrap();
        assert!(plan(
            &white,
            Canvas::new(0, 0, 80, 80),
            &Flow::default(),
            &[[0, 0, 0]],
            64
        )
        .is_empty());

        // A dark band with something to follow, red on its left half and blue on its right:
        // two groups, one for each swatch. A field of flat colour has no form and is left
        // alone, which is what the line above tests.
        let band = band(80);
        let colour: Vec<[u8; 3]> = (0..80 * 80)
            .map(|at| {
                if (at % 80) < 40 {
                    [200, 20, 20]
                } else {
                    [20, 20, 200]
                }
            })
            .collect();
        let picture = band.with_colour(colour);
        let groups = plan(
            &picture,
            Canvas::new(0, 0, 80, 80),
            &Flow::default(),
            &[[220, 0, 0], [0, 0, 220]],
            64,
        );
        assert_eq!(groups.len(), 2, "one group for each swatch that was used");
    }

    /// A picture with two dark bands far apart: given ink for roughly one band's worth of
    /// line, the lines taken should speak about both rather than pile onto one.
    fn two_bands(size: u32) -> Picture {
        let grey = (0..size * size)
            .map(|at| {
                let y = at / size;
                let near = y > size / 8 && y < size / 4;
                let far = y > size * 3 / 4 && y < size * 7 / 8;
                if near || far {
                    20
                } else {
                    250
                }
            })
            .collect();
        Picture::from_grey(grey, size, size).unwrap()
    }

    #[test]
    fn a_budget_in_seconds_buys_the_lines_that_say_the_most() {
        let picture = two_bands(160);
        let canvas = Canvas::new(0, 0, 160, 160);
        let hurried = Flow {
            seconds: 2.0,
            speed: 950.0,
            per_command: 0.11,
            ..Flow::default()
        };
        let groups = plan(&picture, canvas, &hurried, &[[0, 0, 0]], 64);
        let lines: Vec<_> = groups.iter().flat_map(|(_, plan)| &plan.strokes).collect();
        assert!(!lines.is_empty(), "some line is worth drawing");

        // Two seconds buys the drawing, the travel between the lines and a command each.
        let drawn: f32 = lines.iter().map(|line| length_of(line)).sum();
        let commands = lines.len() as f32 * 0.11;
        assert!(
            drawn / 950.0 + commands <= 2.0,
            "it spends the time it was given, not more: {drawn} pixels and {commands}s of commands"
        );

        // Both bands are spoken about: no line falls between them, so every line sits in one
        // or the other, and neither is left out.
        let (mut near, mut far) = (0, 0);
        for line in &lines {
            let middle = line[line.len() / 2].y;
            if middle < 80 {
                near += 1;
            } else {
                far += 1;
            }
        }
        assert!(
            near > 0 && far > 0,
            "the ink is spread over what has not been said: {near} near, {far} far"
        );
    }

    #[test]
    fn the_dark_is_gone_over_again_and_the_light_is_not() {
        let picture = two_bands(160);
        let canvas = Canvas::new(0, 0, 160, 160);
        let flat = Flow {
            seconds: 6.0,
            ..Flow::default()
        };
        let heavy = Flow {
            weight: 3,
            ..flat.clone()
        };
        let count = |flow: &Flow| -> usize {
            plan(&picture, canvas, flow, &[[0, 0, 0]], 64)
                .iter()
                .map(|(_, plan)| plan.strokes.len())
                .sum()
        };
        assert!(
            count(&heavy) > count(&flat),
            "going over the dark lines again makes more lines to draw: {} against {}",
            count(&heavy),
            count(&flat)
        );

        // A second pass is beside the first, not on top of it.
        let line = vec![
            StrokePoint { x: 10, y: 10 },
            StrokePoint { x: 10, y: 40 },
            StrokePoint { x: 10, y: 70 },
        ];
        let over = beside(&line, 1);
        assert_eq!(over.len(), line.len(), "the same line, moved");
        assert!(
            over.iter().zip(&line).all(|(a, b)| a != b),
            "every point of it has moved"
        );
        assert!(
            over.iter().zip(&line).all(|(a, b)| (a.y - b.y).abs() <= 1),
            "across the line, not along it"
        );
    }

    #[test]
    fn the_lines_are_put_in_an_order_that_walks_between_them() {
        let picture = two_bands(160);
        let canvas = Canvas::new(0, 0, 160, 160);
        let groups = plan(&picture, canvas, &Flow::default(), &[[0, 0, 0]], 64);
        let lines: Vec<_> = groups
            .iter()
            .flat_map(|(_, plan)| plan.strokes.clone())
            .collect();
        assert!(
            lines.len() > 4,
            "enough lines to have an order: {}",
            lines.len()
        );

        let walked = |order: &[Vec<StrokePoint>]| -> f32 {
            order
                .windows(2)
                .map(|pair| {
                    let from = pair[0][pair[0].len() - 1];
                    let to = pair[1][0];
                    let (dx, dy) = ((to.x - from.x) as f32, (to.y - from.y) as f32);
                    (dx * dx + dy * dy).sqrt()
                })
                .sum()
        };
        let mut scattered = lines.clone();
        scattered.sort_by_key(|line| line[0].x);
        let mut walk = lines.clone();
        shortest_way_round(&mut walk);
        assert!(
            walked(&walk) <= walked(&scattered),
            "walking between them is no further than any other order: {} against {}",
            walked(&walk),
            walked(&scattered)
        );
    }

    #[test]
    fn when_there_is_time_for_only_a_few_lines_they_are_the_longest() {
        let picture = band(120);
        let canvas = Canvas::new(0, 0, 120, 120);
        let all = plan(&picture, canvas, &Flow::default(), &[[0, 0, 0]], 64);
        let drawn: usize = all.iter().map(|(_, plan)| plan.strokes.len()).sum();
        assert!(drawn > 4, "there is something to choose from: {drawn}");

        let few = Flow {
            most: 3,
            ..Flow::default()
        };
        let kept = plan(&picture, canvas, &few, &[[0, 0, 0]], 64);
        let lines: Vec<_> = kept.iter().flat_map(|(_, plan)| &plan.strokes).collect();
        assert_eq!(lines.len(), 3, "three lines, as asked");

        let shortest_kept = lines
            .iter()
            .map(|line| length_of(line))
            .fold(f32::MAX, f32::min);
        let longest_dropped = all
            .iter()
            .flat_map(|(_, plan)| &plan.strokes)
            .map(|line| length_of(line))
            .fold(Vec::new(), |mut all, one| {
                all.push(one);
                all
            });
        let mut ordered = longest_dropped;
        ordered.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        assert!(
            shortest_kept >= ordered[2] - 0.01,
            "the three kept are the three longest: kept from {shortest_kept}, third longest {}",
            ordered[2]
        );
    }
}
