//! Is a corpus teachable? — a look at what `archie-teach` collected, before blaming the
//! model for not learning it.
//!
//! It answers three questions with numbers rather than opinion:
//!   * do the labelled boxes sit on anything? (edge energy inside them against the tile's)
//!   * are they in the right place? (the same measure over a grid of shifts: a peak away
//!     from zero is a coordinate bug, and says exactly how far off)
//!   * what shape are they? (sizes in tile pixels)
//!
//! It also writes the first few tiles as pictures with the boxes drawn on them, because a
//! corpus that is wrong is usually obviously wrong once seen.
//!
//! Run: `cargo run -p archie-reflex --example corpus_check -- <corpus dir> [out dir]`

use archie_reflex::{Corpus, Sample, TILE_HEIGHT, TILE_WIDTH};

/// How much a pixel differs from its neighbours: where a control's edges are.
fn edges(pixels: &[u8]) -> Vec<f32> {
    let mut out = vec![0.0; pixels.len()];
    for y in 0..TILE_HEIGHT {
        for x in 0..TILE_WIDTH {
            let at = y * TILE_WIDTH + x;
            let here = f32::from(pixels[at]);
            let right = if x + 1 < TILE_WIDTH {
                f32::from(pixels[at + 1])
            } else {
                here
            };
            let below = if y + 1 < TILE_HEIGHT {
                f32::from(pixels[at + TILE_WIDTH])
            } else {
                here
            };
            out[at] = (right - here).abs() + (below - here).abs();
        }
    }
    out
}

/// The mean edge energy inside every labelled box, with the labels moved by (dx, dy).
fn inside(samples: &[Sample], energies: &[Vec<f32>], dx: f32, dy: f32) -> f32 {
    let mut total = 0.0;
    let mut counted = 0_u64;
    for (sample, energy) in samples.iter().zip(energies) {
        for control in &sample.controls {
            let x0 = (control.x + dx).round().max(0.0) as usize;
            let y0 = (control.y + dy).round().max(0.0) as usize;
            let x1 = ((control.x + dx + control.width).round().max(0.0) as usize).min(TILE_WIDTH);
            let y1 = ((control.y + dy + control.height).round().max(0.0) as usize).min(TILE_HEIGHT);
            for y in y0..y1 {
                for x in x0..x1 {
                    total += energy[y * TILE_WIDTH + x];
                    counted += 1;
                }
            }
        }
    }
    if counted == 0 {
        0.0
    } else {
        total / counted as f32
    }
}

/// One tile, three times the size, with its labels drawn around what they claim.
fn drawn(sample: &Sample, path: &std::path::Path) -> std::io::Result<()> {
    const SCALE: usize = 3;
    let (width, height) = (TILE_WIDTH * SCALE, TILE_HEIGHT * SCALE);
    let mut rgb = vec![0_u8; width * height * 3];
    for y in 0..height {
        for x in 0..width {
            let grey = sample.tile.pixels()[(y / SCALE) * TILE_WIDTH + x / SCALE];
            let at = (y * width + x) * 3;
            rgb[at] = grey;
            rgb[at + 1] = grey;
            rgb[at + 2] = grey;
        }
    }
    for control in &sample.controls {
        let x0 = ((control.x.max(0.0) as usize) * SCALE).min(width - 1);
        let y0 = ((control.y.max(0.0) as usize) * SCALE).min(height - 1);
        let x1 = (((control.x + control.width).max(0.0) as usize) * SCALE).min(width - 1);
        let y1 = (((control.y + control.height).max(0.0) as usize) * SCALE).min(height - 1);
        let mut mark = |x: usize, y: usize| {
            let at = (y * width + x) * 3;
            rgb[at] = 255;
            rgb[at + 1] = 32;
            rgb[at + 2] = 32;
        };
        for x in x0..=x1 {
            mark(x, y0);
            mark(x, y1);
        }
        for y in y0..=y1 {
            mark(x0, y);
            mark(x1, y);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgb)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .ok_or("usage: corpus_check <corpus dir> [out dir]")?;
    let out = args.next();
    let samples = Corpus::at(&dir).read()?;
    if samples.is_empty() {
        return Err("the corpus is empty".into());
    }
    let energies: Vec<Vec<f32>> = samples.iter().map(|s| edges(s.tile.pixels())).collect();

    let controls: usize = samples.iter().map(|s| s.controls.len()).sum();
    let flat = samples
        .iter()
        .filter(|s| {
            let pixels = s.tile.pixels();
            pixels.iter().max() == pixels.iter().min()
        })
        .count();
    let everywhere: f32 = energies
        .iter()
        .map(|energy| energy.iter().sum::<f32>() / energy.len() as f32)
        .sum::<f32>()
        / samples.len() as f32;
    println!(
        "{} windows, {controls} controls, {flat} tiles with nothing in them",
        samples.len()
    );
    println!("mean edge energy over whole tiles: {everywhere:.2}");
    println!(
        "mean edge energy inside the labels: {:.2}",
        inside(&samples, &energies, 0.0, 0.0)
    );

    let mut sizes: Vec<(f32, f32)> = samples
        .iter()
        .flat_map(|s| s.controls.iter().map(|c| (c.width, c.height)))
        .collect();
    sizes.sort_by(|a, b| (a.0 * a.1).partial_cmp(&(b.0 * b.1)).unwrap());
    if let (Some(small), Some(large)) = (sizes.first(), sizes.last()) {
        let middle = sizes[sizes.len() / 2];
        println!(
            "box sizes in tile pixels: smallest {:.1}x{:.1}, middle {:.1}x{:.1}, largest {:.1}x{:.1}",
            small.0, small.1, middle.0, middle.1, large.0, large.1
        );
    }

    // One cell can only teach one control. How many does each grid size throw away?
    for cell in [8_usize, 6, 4, 3, 2] {
        let mut shared = 0;
        for sample in &samples {
            let mut taken: Vec<(usize, usize)> = Vec::new();
            for control in &sample.controls {
                let (cx, cy) = control.centre();
                let at = (
                    (cx as usize / cell).min(TILE_WIDTH / cell - 1),
                    (cy as usize / cell).min(TILE_HEIGHT / cell - 1),
                );
                if taken.contains(&at) {
                    shared += 1;
                } else {
                    taken.push(at);
                }
            }
        }
        println!(
            "cells of {cell}px: {shared} of {controls} controls share a cell with another ({}/1000)",
            (shared as f32 / controls as f32 * 1_000.0) as u32
        );
    }

    println!("edge energy inside the labels, moved (rows are dy, columns dx):");
    print!("        ");
    for dx in (-24..=24).step_by(8) {
        print!("{dx:>8}");
    }
    println!();
    for dy in (-24..=24).step_by(8) {
        print!("{dy:>8}");
        for dx in (-24..=24).step_by(8) {
            print!("{:>8.2}", inside(&samples, &energies, dx as f32, dy as f32));
        }
        println!();
    }

    if let Some(out) = out {
        std::fs::create_dir_all(&out)?;
        for (index, sample) in samples.iter().take(6).enumerate() {
            let path = std::path::Path::new(&out).join(format!("sample-{index}.png"));
            drawn(sample, &path)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}
