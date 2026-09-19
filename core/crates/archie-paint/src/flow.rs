//! Lines that follow the form.
//!
//! Hatching is fast to plan and honest about tone, and it looks like hatching: a grid laid
//! over a face. A hand does something else — it draws *along* what it sees, so the line
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
/// `palette` is what the application can be told to draw with — its swatches. Each line
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
    if flow.most > 0 && drawn.len() > flow.most {
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

    let mut by_colour: Vec<Plan> = vec![Plan::default(); palette.len().max(1)];
    for (at, stroke) in drawn {
        by_colour[at].strokes.push(stroke);
    }
    by_colour
        .into_iter()
        .enumerate()
        .filter(|(_, plan)| !plan.is_empty())
        .collect()
}

/// How far the pointer travels along a line.
fn length_of(stroke: &[StrokePoint]) -> f32 {
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
        assert!(plan(&white, Canvas::new(0, 0, 80, 80), &Flow::default(), &[[0, 0, 0]], 64).is_empty());

        // A dark band with something to follow, red on its left half and blue on its right:
        // two groups, one for each swatch. A field of flat colour has no form and is left
        // alone, which is what the line above tests.
        let band = band(80);
        let colour: Vec<[u8; 3]> = (0..80 * 80)
            .map(|at| if (at % 80) < 40 { [200, 20, 20] } else { [20, 20, 200] })
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

