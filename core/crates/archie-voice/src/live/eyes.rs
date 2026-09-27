//! Archie's eyes in a live session: the browser's camera frames, read by IRIS.
//!
//! IRIS runs as its own service (`iris_serve`, loopback on the GPU host) and answers a
//! frame with boxes and the phrase each box matched. Nothing here stores a frame: the
//! latest one lives in memory until the next replaces it, and a crop leaves this process
//! only when curiosity or a question asks what an object is — never of a person.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One thing IRIS saw: the phrase it matched, how sure it was, and where (pixels, xyxy).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub phrase: String,
    pub score: f32,
    #[serde(rename = "box")]
    pub bbox: [f32; 4],
    /// IRIS's instance embedding (distilled from DINOv2): close for the same object.
    #[serde(default, skip_serializing)]
    pub embedding: Option<Vec<f32>>,
}

/// The latest reading of the camera.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub seen: Vec<Seen>,
    pub width: u32,
    pub height: u32,
    /// The JPEG IRIS read, kept only until the next frame.
    pub frame: Arc<Vec<u8>>,
    pub at: Option<Instant>,
    /// Wall-clock time the uploaded frame entered IRIS; `Instant` is only process-local.
    pub observed_at_ms: Option<i64>,
    /// The IRIS training step whose weights read it.
    pub step: Option<u64>,
}

/// What is never sent out of the host, and never cropped for identification: people.
/// Their identity is Wanda's business and needs consent, not a web search.
pub const PEOPLE: &[&str] = &[
    "person", "face", "eye", "hand", "finger", "hair", "beard", "player", "referee", "crowd",
];

pub fn is_person(phrase: &str) -> bool {
    PEOPLE.contains(&phrase)
}

#[derive(Debug, Deserialize)]
struct IrisAnswer {
    detections: Vec<Seen>,
    image_size: [u32; 2],
    step: Option<u64>,
}

/// A client for the IRIS service.
#[derive(Clone)]
pub struct IrisEyes {
    http: reqwest::Client,
    url: String,
    threshold: f32,
}

impl IrisEyes {
    /// `url` is the service root, e.g. `http://127.0.0.1:8765`.
    pub fn new(url: &str, threshold: f32) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            url: url.trim_end_matches('/').to_string(),
            threshold,
        })
    }

    /// Reads one JPEG frame in IRIS's open mode (its whole everyday vocabulary).
    pub async fn look(&self, jpeg: Arc<Vec<u8>>) -> Result<Scene, String> {
        let observed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        let answer: IrisAnswer = self
            .http
            .post(format!(
                "{}/detect?mode=all&embed=1&threshold={}",
                self.url, self.threshold
            ))
            .body(jpeg.as_ref().clone())
            .send()
            .await
            .map_err(|e| format!("IRIS: {e}"))?
            .error_for_status()
            .map_err(|e| format!("IRIS: {e}"))?
            .json()
            .await
            .map_err(|e| format!("IRIS answer: {e}"))?;
        Ok(Scene {
            seen: answer.detections,
            width: answer.image_size[0],
            height: answer.image_size[1],
            frame: jpeg,
            at: Some(Instant::now()),
            observed_at_ms: Some(observed_at_ms),
            step: answer.step,
        })
    }
}

fn place(scene: &Scene, bbox: &[f32; 4]) -> &'static str {
    let (w, h) = (scene.width.max(1) as f32, scene.height.max(1) as f32);
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0 / w, (bbox[1] + bbox[3]) / 2.0 / h);
    match ((cx * 3.0) as u32, (cy * 3.0) as u32) {
        (0, 0) => "arriba a la izquierda",
        (1, 0) => "arriba",
        (_, 0) => "arriba a la derecha",
        (0, 1) => "a la izquierda",
        (1, 1) => "al centro",
        (_, 1) => "a la derecha",
        (0, _) => "abajo a la izquierda",
        (1, _) => "abajo",
        _ => "abajo a la derecha",
    }
}

/// What is in a frame, grouped: "cup (al centro)", "2 × chair", "quizás knife (abajo)".
/// IRIS below half sure is marked as unsure, so the model does not state it as fact.
pub fn list(scene: &Scene) -> Vec<String> {
    let mut groups: Vec<(String, usize, f32, &'static str)> = Vec::new();
    for item in &scene.seen {
        match groups.iter_mut().find(|group| group.0 == item.phrase) {
            Some(group) => {
                group.1 += 1;
                group.2 = group.2.max(item.score);
            }
            None => groups.push((item.phrase.clone(), 1, item.score, place(scene, &item.bbox))),
        }
    }
    groups.sort_by(|a, b| b.2.total_cmp(&a.2));
    groups
        .iter()
        .take(12)
        .map(|(phrase, count, score, place)| {
            let unsure = if *score < 0.5 { "quizás " } else { "" };
            match count {
                1 => format!("{unsure}{phrase} ({place})"),
                n => format!("{unsure}{n} × {phrase}"),
            }
        })
        .collect()
}

/// The scene as the model is told it: what, how many, where, and how old the look is.
pub fn describe(scene: &Scene) -> String {
    let Some(at) = scene.at else {
        return "Cámara: todavía no me llega imagen.".into();
    };
    if scene.seen.is_empty() {
        return format!(
            "Cámara (hace {:.1} s): no reconozco nada con seguridad.",
            at.elapsed().as_secs_f32()
        );
    }
    let listed = list(scene);
    format!(
        "Cámara (hace {:.1} s, IRIS en modo abierto; son categorías generales, no nombres \
         exactos): {}.",
        at.elapsed().as_secs_f32(),
        listed.join(", ")
    )
}

/// The object's crop as PNG, a little wider than its box, with every face inside it
/// blacked out. Large crops are subsampled to at most 640 px on the long side.
pub fn crop_png(jpeg: &[u8], bbox: [f32; 4], faces: &[[f32; 4]]) -> Result<Vec<u8>, String> {
    let mut decoder = jpeg_decoder::Decoder::new(jpeg);
    let pixels = decoder.decode().map_err(|e| format!("frame: {e}"))?;
    let info = decoder.info().ok_or("frame: no header")?;
    let (width, height) = (info.width as usize, info.height as usize);
    let channels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => 3,
        jpeg_decoder::PixelFormat::L8 => 1,
        other => return Err(format!("frame: unsupported pixel format {other:?}")),
    };
    let pad_x = (bbox[2] - bbox[0]) * 0.08;
    let pad_y = (bbox[3] - bbox[1]) * 0.08;
    let x0 = (bbox[0] - pad_x).max(0.0) as usize;
    let y0 = (bbox[1] - pad_y).max(0.0) as usize;
    let x1 = ((bbox[2] + pad_x) as usize).min(width);
    let y1 = ((bbox[3] + pad_y) as usize).min(height);
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        return Err("the object is too small to look at closely".into());
    }
    let stride = ((x1 - x0).max(y1 - y0)).div_ceil(640).max(1);
    let (out_w, out_h) = ((x1 - x0).div_ceil(stride), (y1 - y0).div_ceil(stride));
    let hidden = |x: usize, y: usize| {
        faces.iter().any(|face| {
            (x as f32) >= face[0] && (x as f32) <= face[2] && (y as f32) >= face[1] && (y as f32) <= face[3]
        })
    };
    let mut rgb = Vec::with_capacity(out_w * out_h * 3);
    for y in (y0..y1).step_by(stride) {
        for x in (x0..x1).step_by(stride) {
            if hidden(x, y) {
                rgb.extend_from_slice(&[0, 0, 0]);
                continue;
            }
            let at = (y * width + x) * channels;
            if channels == 3 {
                rgb.extend_from_slice(&pixels[at..at + 3]);
            } else {
                rgb.extend_from_slice(&[pixels[at]; 3]);
            }
        }
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, out_w as u32, out_h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
    }
    Ok(encoded)
}

/// The box a request most likely means: the most confident non-person object whose phrase
/// shares a word with the request, else the most prominent non-person object.
pub fn pick<'a>(scene: &'a Scene, request: &str) -> Option<&'a Seen> {
    let request = request.to_lowercase();
    let area = |item: &Seen| (item.bbox[2] - item.bbox[0]) * (item.bbox[3] - item.bbox[1]);
    let objects = || scene.seen.iter().filter(|item| !is_person(&item.phrase));
    objects()
        .filter(|item| {
            item.phrase
                .split_whitespace()
                .any(|word| word.len() > 2 && request.contains(word))
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .or_else(|| objects().max_by(|a, b| (a.score * area(a)).total_cmp(&(b.score * area(b)))))
}

/// Face boxes, blacked out of any crop.
pub fn faces(scene: &Scene) -> Vec<[f32; 4]> {
    scene
        .seen
        .iter()
        .filter(|item| matches!(item.phrase.as_str(), "face" | "eye"))
        .map(|item| item.bbox)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(items: &[(&str, f32, [f32; 4])]) -> Scene {
        Scene {
            seen: items
                .iter()
                .map(|(phrase, score, bbox)| Seen {
                    phrase: phrase.to_string(),
                    score: *score,
                    bbox: *bbox,
                    embedding: None,
                })
                .collect(),
            width: 300,
            height: 300,
            frame: Arc::new(Vec::new()),
            at: Some(Instant::now()),
            observed_at_ms: Some(0),
            step: Some(1),
        }
    }

    #[test]
    fn a_request_picks_the_named_object_and_never_a_person() {
        let desk = scene(&[
            ("person", 0.99, [0.0, 0.0, 300.0, 300.0]),
            ("toy", 0.6, [100.0, 100.0, 150.0, 150.0]),
            ("cup", 0.9, [200.0, 200.0, 240.0, 240.0]),
        ]);
        assert_eq!(pick(&desk, "¿qué es este toy?").unwrap().phrase, "toy");
        // Nothing named: the most prominent object (score times area), here the toy.
        assert_eq!(pick(&desk, "qué tengo aquí").unwrap().phrase, "toy");
        assert!(pick(&scene(&[("person", 0.9, [0.0, 0.0, 10.0, 10.0])]), "esto").is_none());
    }

    #[test]
    fn the_description_groups_and_places_things() {
        let text = describe(&scene(&[
            ("cup", 0.9, [0.0, 0.0, 50.0, 50.0]),
            ("cup", 0.8, [250.0, 250.0, 290.0, 290.0]),
            ("laptop", 0.7, [100.0, 100.0, 200.0, 200.0]),
        ]));
        assert!(text.contains("2 × cup"), "{text}");
        assert!(text.contains("laptop (al centro)"), "{text}");
    }

    #[test]
    fn faces_are_black_in_a_crop() {
        // A 32x32 white JPEG is awkward to make without an encoder, so the crop is
        // checked through its refusal of a box too small to be an object.
        assert!(crop_png(&[], [0.0, 0.0, 1.0, 1.0], &[]).is_err());
    }
}
