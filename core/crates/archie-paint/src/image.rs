//! A picture to draw: read from a PNG, kept as greys, and nothing else.
//!
//! Colour is not in the pen's vocabulary, so it is dropped as early as possible: what the
//! planner reads is how light each part of the picture is.

/// A picture in greys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    grey: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum PictureError {
    #[error("reading the picture: {0}")]
    Io(#[from] std::io::Error),
    #[error("that is not a PNG this can read: {0}")]
    Png(String),
    #[error("that is not a JPEG this can read: {0}")]
    Jpeg(String),
    #[error("a picture needs a size and as many pixels as it says it has")]
    Shape,
}

impl Picture {
    pub fn from_grey(grey: Vec<u8>, width: u32, height: u32) -> Result<Self, PictureError> {
        if width == 0 || height == 0 || grey.len() as u64 != u64::from(width) * u64::from(height) {
            return Err(PictureError::Shape);
        }
        Ok(Self {
            grey,
            width,
            height,
        })
    }

    /// Reads a picture and keeps its greys. Colour becomes light the way an eye weighs it,
    /// and anything see-through is taken as the white paper it is drawn on.
    ///
    /// PNG or JPEG, told apart by what the file starts with rather than by its name.
    pub fn read(path: &std::path::Path) -> Result<Self, PictureError> {
        let head = {
            use std::io::Read;
            let mut head = [0_u8; 2];
            let mut file = std::fs::File::open(path)?;
            let read = file.read(&mut head)?;
            if read < 2 {
                return Err(PictureError::Shape);
            }
            head
        };
        if head == [0xFF, 0xD8] {
            return Self::read_jpeg(path);
        }
        Self::read_png(path)
    }

    fn read_jpeg(path: &std::path::Path) -> Result<Self, PictureError> {
        let file = std::fs::File::open(path)?;
        let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
        let pixels = decoder
            .decode()
            .map_err(|e| PictureError::Jpeg(e.to_string()))?;
        let info = decoder
            .info()
            .ok_or_else(|| PictureError::Jpeg("no picture in that file".into()))?;
        let channels = match info.pixel_format {
            jpeg_decoder::PixelFormat::L8 => 1,
            jpeg_decoder::PixelFormat::RGB24 => 3,
            other => return Err(PictureError::Jpeg(format!("{other:?} is not read here"))),
        };
        let grey = pixels
            .chunks_exact(channels)
            .map(|pixel| {
                if channels == 1 {
                    pixel[0]
                } else {
                    (0.2126 * f32::from(pixel[0])
                        + 0.7152 * f32::from(pixel[1])
                        + 0.0722 * f32::from(pixel[2]))
                    .round()
                    .clamp(0.0, 255.0) as u8
                }
            })
            .collect();
        Self::from_grey(grey, u32::from(info.width), u32::from(info.height))
    }

    fn read_png(path: &std::path::Path) -> Result<Self, PictureError> {
        let file = std::fs::File::open(path)?;
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder
            .read_info()
            .map_err(|e| PictureError::Png(e.to_string()))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| PictureError::Png("a picture larger than this can hold".into()))?;
        let mut buffer = vec![0; size];
        let frame = reader
            .next_frame(&mut buffer)
            .map_err(|e| PictureError::Png(e.to_string()))?;
        let info = reader.info();
        if info.bit_depth != png::BitDepth::Eight {
            return Err(PictureError::Png(format!(
                "{:?} bits a channel, and this reads eight",
                info.bit_depth
            )));
        }
        let channels = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => return Err(PictureError::Png(format!("{other:?} is not read here"))),
        };
        let pixels = &buffer[..frame.buffer_size()];
        let grey = pixels
            .chunks_exact(channels)
            .map(|pixel| {
                let (light, alpha) = match channels {
                    1 => (f32::from(pixel[0]), 255.0),
                    2 => (f32::from(pixel[0]), f32::from(pixel[1])),
                    _ => (
                        0.2126 * f32::from(pixel[0])
                            + 0.7152 * f32::from(pixel[1])
                            + 0.0722 * f32::from(pixel[2]),
                        if channels == 4 {
                            f32::from(pixel[3])
                        } else {
                            255.0
                        },
                    ),
                };
                // Over white paper: what is not there is not drawn.
                let over = light * (alpha / 255.0) + 255.0 * (1.0 - alpha / 255.0);
                over.round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Self::from_grey(grey, frame.width, frame.height)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// How light one pixel is, or nothing when the point is off the picture.
    pub fn grey(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.grey
            .get((y as usize) * (self.width as usize) + x as usize)
            .copied()
    }

    /// The same picture with its grain taken out: the average of a square that many pixels
    /// either way.
    ///
    /// A photograph thresholded as it is breaks into speckle, and a pen drawing speckle
    /// lifts every few pixels. Softening first is what turns a tone into a line.
    pub fn softened(&self, radius: u32) -> Self {
        if radius == 0 {
            return self.clone();
        }
        let (width, height) = (self.width as i64, self.height as i64);
        let span = radius as i64;
        // Across, then down: the same square, at a fraction of the arithmetic.
        let mut across = vec![0_u8; self.grey.len()];
        for y in 0..height {
            let row = (y * width) as usize;
            let mut total: u32 = 0;
            for x in -span..=span {
                total += u32::from(self.grey[row + x.clamp(0, width - 1) as usize]);
            }
            let window = (span * 2 + 1) as u32;
            for x in 0..width {
                across[row + x as usize] = (total / window) as u8;
                let leaving = (x - span).clamp(0, width - 1) as usize;
                let coming = (x + span + 1).clamp(0, width - 1) as usize;
                total = total + u32::from(self.grey[row + coming])
                    - u32::from(self.grey[row + leaving]);
            }
        }
        let mut down = vec![0_u8; self.grey.len()];
        for x in 0..width {
            let mut total: u32 = 0;
            for y in -span..=span {
                total += u32::from(across[(y.clamp(0, height - 1) * width + x) as usize]);
            }
            let window = (span * 2 + 1) as u32;
            for y in 0..height {
                down[(y * width + x) as usize] = (total / window) as u8;
                let leaving = (y - span).clamp(0, height - 1);
                let coming = (y + span + 1).clamp(0, height - 1);
                total = total + u32::from(across[(coming * width + x) as usize])
                    - u32::from(across[(leaving * width + x) as usize]);
            }
        }
        Self {
            grey: down,
            width: self.width,
            height: self.height,
        }
    }

    /// How dark the picture has to be for a given share of it to be at least that dark.
    ///
    /// Tone levels chosen this way come from the picture rather than from a number: the
    /// same hatching then reads a dim painting and a bright photograph alike, because what
    /// it is told is "cover the darkest fifth", not "cover everything under 100".
    pub fn darkness_at(&self, shares: &[f32]) -> Vec<u8> {
        let mut histogram = [0_u32; 256];
        for light in &self.grey {
            histogram[255 - *light as usize] += 1;
        }
        let total = self.grey.len() as f32;
        shares
            .iter()
            .map(|share| {
                let wanted = (share.clamp(0.0, 1.0) * total) as u32;
                let mut seen = 0_u32;
                // Walking from the darkest down, the level is where that share is reached.
                for darkness in (0..256).rev() {
                    seen += histogram[darkness];
                    if seen >= wanted {
                        return darkness as u8;
                    }
                }
                0
            })
            .collect()
    }

    /// The same picture with its lights and darks pulled apart, so that a photograph whose
    /// tones all sit in the middle still has something to hatch.
    pub fn stretched(&self) -> Self {
        let (Some(darkest), Some(lightest)) = (self.grey.iter().min(), self.grey.iter().max())
        else {
            return self.clone();
        };
        let (low, high) = (f32::from(*darkest), f32::from(*lightest));
        if high - low < 1.0 {
            return self.clone();
        }
        Self {
            grey: self
                .grey
                .iter()
                .map(|light| (((f32::from(*light) - low) / (high - low)) * 255.0).round() as u8)
                .collect(),
            width: self.width,
            height: self.height,
        }
    }
}

impl Picture {
    /// Reads a picture from PNG bytes, for a capture that never touches a file.
    pub fn from_png(bytes: &[u8]) -> Result<Self, PictureError> {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder
            .read_info()
            .map_err(|e| PictureError::Png(e.to_string()))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| PictureError::Png("a picture larger than this can hold".into()))?;
        let mut buffer = vec![0; size];
        let frame = reader
            .next_frame(&mut buffer)
            .map_err(|e| PictureError::Png(e.to_string()))?;
        let info = reader.info();
        let channels = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => return Err(PictureError::Png(format!("{other:?} is not read here"))),
        };
        let grey = buffer[..frame.buffer_size()]
            .chunks_exact(channels)
            .map(|pixel| {
                if channels <= 2 {
                    pixel[0]
                } else {
                    (0.2126 * f32::from(pixel[0])
                        + 0.7152 * f32::from(pixel[1])
                        + 0.0722 * f32::from(pixel[2]))
                    .round()
                    .clamp(0.0, 255.0) as u8
                }
            })
            .collect();
        Self::from_grey(grey, frame.width, frame.height)
    }
}
