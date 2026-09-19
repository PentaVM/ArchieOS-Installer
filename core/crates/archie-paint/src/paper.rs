//! Finding the paper.
//!
//! An application that can be drawn in has somewhere to draw: a white rectangle in the
//! middle of its window, with its tools and panels around it. This finds the largest one,
//! which is what a person's eye does before putting a pen down.
//!
//! It reads a capture and nothing else. It is not a model, it does not name anything, and
//! what it returns is a proposal like any other: the stage gate still has to allow every
//! stroke that goes there.

use crate::{Canvas, Picture};

/// How light a pixel has to be to count as paper.
pub const PAPER: u8 = 232;

/// The largest rectangle of paper in a capture, in the capture's own pixels.
///
/// Rows are walked once, keeping how deep the paper runs above each column, and the widest
/// rectangle that fits under that profile is the answer — the usual way to find the largest
/// rectangle in a picture of two colours.
pub fn paper_in(picture: &Picture, lightest: u8) -> Option<Canvas> {
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    if width == 0 || height == 0 {
        return None;
    }
    let mut deep = vec![0_u32; width];
    let mut best: Option<(u64, Canvas)> = None;
    for y in 0..height {
        for (x, deep) in deep.iter_mut().enumerate() {
            let light = picture.grey(x as u32, y as u32).unwrap_or(0);
            *deep = if light >= lightest { *deep + 1 } else { 0 };
        }
        // The widest rectangle ending at this row, column by column, with a stack of the
        // columns still open.
        let mut open: Vec<(usize, u32)> = Vec::new();
        // One column past the end, with no paper in it, closes whatever is still open.
        for (x, depth) in deep.iter().copied().chain(std::iter::once(0)).enumerate() {
            let mut starts = x;
            while let Some(&(from, tall)) = open.last() {
                if tall <= depth {
                    break;
                }
                open.pop();
                let area = u64::from(tall) * (x - from) as u64;
                let canvas = Canvas::new(
                    from as i32,
                    (y + 1 - tall as usize) as i32,
                    (x - from) as u32,
                    tall,
                );
                if best.as_ref().is_none_or(|(most, _)| area > *most) {
                    best = Some((area, canvas));
                }
                starts = from;
            }
            if open.last().map(|(_, tall)| *tall) != Some(depth) && depth > 0 {
                open.push((starts, depth));
            }
        }
    }
    best.map(|(_, canvas)| canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_rectangle(width: u32, height: u32, at: (u32, u32, u32, u32)) -> Picture {
        let mut grey = vec![60_u8; (width * height) as usize];
        for y in at.1..at.1 + at.3 {
            for x in at.0..at.0 + at.2 {
                grey[(y * width + x) as usize] = 255;
            }
        }
        Picture::from_grey(grey, width, height).unwrap()
    }

    #[test]
    fn it_finds_the_paper_among_the_panels() {
        let window = with_rectangle(200, 160, (40, 20, 110, 90));
        let paper = paper_in(&window, PAPER).expect("a window with paper in it");
        assert_eq!(
            (paper.x, paper.y, paper.width, paper.height),
            (40, 20, 110, 90)
        );
    }

    #[test]
    fn a_window_with_no_paper_in_it_offers_none() {
        let dark = Picture::from_grey(vec![30; 64 * 64], 64, 64).unwrap();
        assert!(paper_in(&dark, PAPER).is_none());
    }

    #[test]
    fn the_larger_of_two_is_the_one_to_draw_on() {
        let mut grey = vec![10_u8; 300 * 200];
        for y in 10..60 {
            for x in 10..60 {
                grey[y * 300 + x] = 250;
            }
        }
        for y in 80..190 {
            for x in 100..280 {
                grey[y * 300 + x] = 250;
            }
        }
        let window = Picture::from_grey(grey, 300, 200).unwrap();
        let paper = paper_in(&window, PAPER).unwrap();
        assert_eq!((paper.x, paper.y), (100, 80));
        assert_eq!((paper.width, paper.height), (180, 110));
    }
}
