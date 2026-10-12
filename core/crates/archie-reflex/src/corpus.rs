//! What an application that declares itself teaches about one that does not.
//!
//! A sample is one window reduced to a tile, and the boxes the application itself gave for
//! the controls in it. Nothing else is kept: no screenshot, no text, no application id, no
//! time. A tile is 160x96 greys, which is too small to read and enough to see that
//! something is a button.
//!
//! The corpus is two files, so that a training run is reproducible and a corpus can be
//! read by anything: the tiles end to end in one file, and one line of JSON per sample in
//! the other.

use crate::{ReflexError, Tile, TILE_HEIGHT, TILE_WIDTH};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// How many samples one corpus holds. A teaching run that wants more starts another.
pub const MAX_SAMPLES: usize = 20_000;

/// What kind of control a box is, in the three kinds this model is asked to tell apart.
/// Everything else an application declares is not a target and is not taught.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Button,
    Input,
    Other,
}

impl ControlKind {
    pub const COUNT: usize = 3;

    pub fn of(kind: archie_iris::NodeKind) -> Self {
        match kind {
            archie_iris::NodeKind::Button => Self::Button,
            archie_iris::NodeKind::Input => Self::Input,
            _ => Self::Other,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Button => 0,
            Self::Input => 1,
            Self::Other => 2,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Button),
            1 => Some(Self::Input),
            2 => Some(Self::Other),
            _ => None,
        }
    }

    pub fn as_node_kind(self) -> archie_iris::NodeKind {
        match self {
            Self::Button => archie_iris::NodeKind::Button,
            Self::Input => archie_iris::NodeKind::Input,
            Self::Other => archie_iris::NodeKind::UnknownObject,
        }
    }
}

/// One control, in the tile's own coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Box2 {
    pub kind: ControlKind,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Box2 {
    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// The same box in a window of that size.
    pub fn in_window(&self, width: u32, height: u32) -> (i32, i32, u32, u32) {
        let sx = width as f32 / TILE_WIDTH as f32;
        let sy = height as f32 / TILE_HEIGHT as f32;
        (
            (self.x * sx).round() as i32,
            (self.y * sy).round() as i32,
            (self.width * sx).round().max(1.0) as u32,
            (self.height * sy).round().max(1.0) as u32,
        )
    }

    /// A control an application declared, brought into the tile.
    pub fn from_window(
        kind: ControlKind,
        bounds: archie_iris::Rect,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let sx = TILE_WIDTH as f32 / width as f32;
        let sy = TILE_HEIGHT as f32 / height as f32;
        let tile = Self {
            kind,
            x: bounds.x as f32 * sx,
            y: bounds.y as f32 * sy,
            width: bounds.width as f32 * sx,
            height: bounds.height as f32 * sy,
        };
        // Something with no size in the tile teaches nothing except noise.
        (tile.width >= 1.0
            && tile.height >= 1.0
            && tile.x >= 0.0
            && tile.y >= 0.0
            && tile.x + tile.width <= TILE_WIDTH as f32 + 1.0
            && tile.y + tile.height <= TILE_HEIGHT as f32 + 1.0)
            .then_some(tile)
    }
}

/// One window and what was declared in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub tile: Tile,
    pub controls: Vec<Box2>,
}

#[derive(Serialize, Deserialize)]
struct Labels {
    source_width: u32,
    source_height: u32,
    controls: Vec<Box2>,
}

/// A corpus on disk, written once and read many times.
pub struct Corpus {
    dir: PathBuf,
}

impl Corpus {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn tiles(&self) -> PathBuf {
        self.dir.join("tiles.bin")
    }

    fn labels(&self) -> PathBuf {
        self.dir.join("labels.jsonl")
    }

    /// Adds one sample. Tiles are all the same size, so the nth tile is at n * its size.
    pub fn append(&self, sample: &Sample) -> Result<(), ReflexError> {
        std::fs::create_dir_all(&self.dir)?;
        let mut tiles = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.tiles())?;
        tiles.write_all(sample.tile.pixels())?;
        let mut labels = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.labels())?;
        let line = serde_json::to_string(&Labels {
            source_width: sample.tile.source_width,
            source_height: sample.tile.source_height,
            controls: sample.controls.clone(),
        })
        .map_err(|_| ReflexError::Corpus)?;
        labels.write_all(line.as_bytes())?;
        labels.write_all(b"\n")?;
        Ok(())
    }

    /// Everything in it, in the order it was taught.
    pub fn read(&self) -> Result<Vec<Sample>, ReflexError> {
        let tiles = std::fs::read(self.tiles())?;
        let labels = std::fs::File::open(self.labels())?;
        let size = TILE_WIDTH * TILE_HEIGHT;
        let mut samples = Vec::new();
        for (index, line) in std::io::BufReader::new(labels).lines().enumerate() {
            if samples.len() >= MAX_SAMPLES {
                break;
            }
            let line = line?;
            let labels: Labels = serde_json::from_str(&line).map_err(|_| ReflexError::Corpus)?;
            let at = index * size;
            let Some(pixels) = tiles.get(at..at + size) else {
                return Err(ReflexError::Corpus);
            };
            samples.push(Sample {
                tile: Tile::from_pixels(
                    pixels.to_vec(),
                    labels.source_width,
                    labels.source_height,
                )?,
                controls: labels.controls,
            });
        }
        Ok(samples)
    }

    pub fn count(&self) -> usize {
        std::fs::File::open(self.labels())
            .map(|file| std::io::BufReader::new(file).lines().count())
            .unwrap_or(0)
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}
