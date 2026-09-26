//! What Archie knows about what it has seen, and has not said.
//!
//! IRIS names things by category — "toy", "car", "bottle". Curiosity looks closer at the
//! prominent object in front of the camera, finds out what exactly it is, reads a little
//! about it, and keeps that in a buffer. The buffer is told to the model as context, marked
//! as known and unsaid: Archie brings it up when asked, or when it answers what was asked.
//! It never volunteers it into silence; the floor decides whether Archie speaks at all.
//!
//! Finding out what an object is, in order of preference:
//!
//! 1. **Web detection** (`GOOGLE_VISION_API_KEY`): Google Cloud Vision's `WEB_DETECTION`,
//!    the engine behind Lens. It returns a best guess ("froakie plush") and matching pages.
//!    It is a search, not a generative model, so it does not invent an answer.
//! 2. **A vision model on Nebius** (`NEBIUS_TOKEN_FACTORY_KEY` and `ARCHIE_LIVE_VLM`),
//!    asked only when web detection has no confident guess or is not configured.
//! 3. Neither: IRIS's own category, said as a category.
//!
//! What leaves the host is a crop of the object with faces blacked out, and never a crop
//! of a person. The query sent to Tavily is only the object's name.

use crate::live::eyes::{self, Scene, Seen};
use archie_search::{
    SearchAdmission, SearchAuthority, SearchDepth, SearchGate, SearchOptions, SearchProposal,
    SearchProviderManifest, SearchRegistry, SearchTopic, TavilyClient,
};
use archie_types::capability::well_known;
use archie_types::PrivacyGrade;
use archie_vault::{EntryClass, Vault, VaultKey, VaultRef};
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, Zeroizing};

/// One thing found out.
#[derive(Debug, Clone, Serialize)]
pub struct Known {
    /// IRIS's category for it.
    pub seen_as: String,
    /// What it is exactly, as best found.
    pub name: String,
    /// Who said so: `web`, `vision-model` or `iris`.
    pub by: String,
    /// A few sentences read about it, when a search was made.
    pub facts: String,
    pub sources: Vec<String>,
    #[serde(skip)]
    pub bbox: [f32; 4],
    #[serde(skip)]
    pub at: Option<Instant>,
}

/// The last few things found out, newest first.
#[derive(Default)]
pub struct Buffer {
    entries: VecDeque<Known>,
}

impl Buffer {
    const CAPACITY: usize = 16;

    pub fn remember(&mut self, known: Known) {
        self.entries
            .retain(|entry| !entry.name.eq_ignore_ascii_case(&known.name));
        self.entries.push_front(known);
        self.entries.truncate(Self::CAPACITY);
    }

    pub fn entries(&self) -> impl Iterator<Item = &Known> {
        self.entries.iter()
    }

    /// Something already looked at: the same category, overlapping, in the last minutes.
    pub fn covers(&self, seen: &Seen) -> bool {
        self.entries.iter().any(|entry| {
            entry.seen_as == seen.phrase
                && iou(&entry.bbox, &seen.bbox) > 0.25
                && entry.at.is_some_and(|at| at.elapsed() < Duration::from_secs(180))
        })
    }

    /// The buffer as the model is told it.
    pub fn describe(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = self
            .entries
            .iter()
            .take(8)
            .map(|entry| {
                let mut line = format!("- lo que IRIS ve como «{}» es: {}", entry.seen_as, entry.name);
                if !entry.facts.is_empty() {
                    line.push_str(&format!(". Datos: {}", entry.facts));
                }
                line
            })
            .collect();
        format!(
            "Lo que ya averiguaste mirando de cerca (no lo digas por decirlo: úsalo si te \
             preguntan o si responde lo que te preguntan):\n{}",
            lines.join("\n")
        )
    }
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    inter / (area(a) + area(b) - inter).max(1e-6)
}

/// A best guess at what an object is.
#[derive(Debug, Clone, PartialEq)]
pub struct Identified {
    pub name: String,
    pub by: &'static str,
    pub pages: Vec<String>,
}

/// Finds out what an object is from its crop.
pub struct Identifier {
    http: reqwest::Client,
    google: Option<Zeroizing<String>>,
    nebius: Option<(Zeroizing<String>, String)>,
}

impl Identifier {
    /// Reads the keys from the environment; any may be missing.
    pub fn from_environment() -> Result<Self, String> {
        let key = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| Zeroizing::new(value.trim().to_string()))
                .filter(|value| !value.is_empty())
        };
        let nebius = match (key("NEBIUS_TOKEN_FACTORY_KEY"), std::env::var("ARCHIE_LIVE_VLM").ok()) {
            (Some(key), Some(model)) if !model.trim().is_empty() => Some((key, model.trim().to_string())),
            _ => None,
        };
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| e.to_string())?,
            google: key("GOOGLE_VISION_API_KEY"),
            nebius,
        })
    }

    /// Which ways of finding out are configured, for the page and the log.
    pub fn means(&self) -> Vec<&'static str> {
        let mut means = Vec::new();
        if self.google.is_some() {
            means.push("web");
        }
        if self.nebius.is_some() {
            means.push("vision-model");
        }
        means
    }

    pub async fn identify(&self, png: &[u8], category: &str) -> Result<Option<Identified>, String> {
        if let Some(key) = &self.google {
            match self.web_detection(key, png).await {
                Ok(Some(found)) => return Ok(Some(found)),
                Ok(None) => {}
                Err(error) if self.nebius.is_none() => return Err(error),
                Err(_) => {}
            }
        }
        if let Some((key, model)) = &self.nebius {
            return self.vision_model(key, model, png, category).await;
        }
        Ok(None)
    }

    async fn web_detection(&self, key: &str, png: &[u8]) -> Result<Option<Identified>, String> {
        let body = serde_json::json!({"requests": [{
            "image": {"content": base64::engine::general_purpose::STANDARD.encode(png)},
            "features": [{"type": "WEB_DETECTION", "maxResults": 8}],
        }]});
        let answer: serde_json::Value = self
            .http
            .post("https://vision.googleapis.com/v1/images:annotate")
            // In a header, so the key never appears in a URL or a proxy's log.
            .header("x-goog-api-key", key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("web detection: {e}"))?
            .error_for_status()
            .map_err(|e| format!("web detection: {}", e.status().map_or("failed".into(), |s| s.to_string())))?
            .json()
            .await
            .map_err(|e| format!("web detection answer: {e}"))?;
        let web = &answer["responses"][0]["webDetection"];
        let guess = web["bestGuessLabels"][0]["label"].as_str().map(str::to_string);
        let entity = web["webEntities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entity| entity["score"].as_f64().unwrap_or(0.0) >= 0.6)
            .find_map(|entity| entity["description"].as_str().map(str::to_string));
        let pages = web["pagesWithMatchingImages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|page| page["url"].as_str().map(str::to_string))
            .take(3)
            .collect();
        Ok(match (guess, entity) {
            (Some(guess), Some(entity)) if !guess.to_lowercase().contains(&entity.to_lowercase()) => {
                Some(Identified { name: format!("{guess} ({entity})"), by: "web", pages })
            }
            (Some(name), _) | (None, Some(name)) => Some(Identified { name, by: "web", pages }),
            (None, None) => None,
        })
    }

    async fn vision_model(
        &self,
        key: &str,
        model: &str,
        png: &[u8],
        category: &str,
    ) -> Result<Option<Identified>, String> {
        let image = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        let body = serde_json::json!({
            "model": model,
            "temperature": 0.0,
            "max_tokens": 40,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": format!(
                    "A detector labelled this object \"{category}\". What exactly is it? Answer \
                     with only its specific name (brand and model, character, species, product), \
                     in at most eight words. If you cannot tell, answer exactly: unknown")},
                {"type": "image_url", "image_url": {"url": image}},
            ]}],
        });
        let answer: serde_json::Value = self
            .http
            .post("https://api.tokenfactory.nebius.com/v1/chat/completions")
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("vision model: {e}"))?
            .error_for_status()
            .map_err(|e| format!("vision model: {}", e.status().map_or("failed".into(), |s| s.to_string())))?
            .json()
            .await
            .map_err(|e| format!("vision model answer: {e}"))?;
        let text = answer["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_matches(|c: char| c == '.' || c == '"')
            .to_string();
        Ok((!text.is_empty() && !text.eq_ignore_ascii_case("unknown")).then(|| Identified {
            name: text,
            by: "vision-model",
            pages: Vec::new(),
        }))
    }
}

/// Reads about a named thing through Archie's search gate (Tavily).
pub struct Researcher {
    manifest: SearchProviderManifest,
    registry: SearchRegistry,
    vault: Mutex<Vault>,
    reference: VaultRef,
}

const ACTOR: &str = "archie-voice-live";

impl Researcher {
    /// `None` when `TAVILY_API_KEY` is not set. The key is sealed into a vault that lives
    /// only in this process, as `archie-search` does.
    pub fn from_environment() -> Result<Option<Self>, String> {
        let Ok(raw) = std::env::var("TAVILY_API_KEY") else {
            return Ok(None);
        };
        let key = Zeroizing::new(raw.trim().to_string());
        if key.is_empty() {
            return Ok(None);
        }
        let reference = VaultRef::parse("vault:tavily/main").map_err(|e| e.to_string())?;
        let manifest = SearchProviderManifest::tavily(reference.clone());
        let mut registry = SearchRegistry::new();
        registry.register(manifest.clone()).map_err(|e| e.to_string())?;
        let mut seed = [0_u8; 32];
        SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| "the system random source failed".to_string())?;
        let vault_key = VaultKey::from_bytes(seed);
        seed.zeroize();
        let mut vault = Vault::open(&vault_key).map_err(|e| e.to_string())?;
        vault
            .store(
                ACTOR,
                reference.clone(),
                EntryClass::Credential,
                PrivacyGrade::Secret,
                key.as_str(),
                now_ms(),
                None,
            )
            .map_err(|e| e.to_string())?;
        Ok(Some(Self {
            manifest,
            registry,
            vault: Mutex::new(vault),
            reference,
        }))
    }

    /// Up to three results for `query`, as (title, url, snippet). The query must be
    /// public: it leaves the machine.
    pub async fn search(&self, query: &str) -> Result<Vec<(String, String, String)>, String> {
        let authority = SearchAuthority::new([well_known::network_internet()].into_iter().collect());
        let proposal = SearchProposal::new(&self.manifest.id, query, PrivacyGrade::Public, "live", "live")
            .with_options(SearchOptions {
                depth: SearchDepth::Basic,
                topic: SearchTopic::General,
                max_results: 3,
                ..SearchOptions::default()
            });
        let search = match SearchGate::authorize(&self.registry, &authority, &proposal) {
            SearchAdmission::Allowed(search) => search,
            SearchAdmission::NeedsApproval { reason, .. } => return Err(format!("needs approval: {reason}")),
            SearchAdmission::Refused(refusal) => return Err(format!("refused: {refusal}")),
        };
        let credential = self
            .vault
            .lock()
            .map_err(|_| "the vault lock is poisoned".to_string())?
            .broker_credential(ACTOR, &self.reference, now_ms())
            .map_err(|e| e.to_string())?;
        let client = TavilyClient::connect(&self.manifest).map_err(|e| e.to_string())?;
        let results = client
            .search(&search, credential)
            .await
            .map_err(|e| format!("{e} ({})", e.code()))?;
        Ok(results
            .results
            .into_iter()
            .map(|result| (result.title, result.url, result.snippet))
            .collect())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Snippets cut to something a voice can say: the first two, 280 characters each.
pub fn facts_from(results: &[(String, String, String)]) -> String {
    results
        .iter()
        .take(2)
        .map(|(_, _, snippet)| {
            let text: String = snippet.chars().take(280).collect();
            text.trim().to_string()
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Looks closer at `seen` in `scene`: identify it, read about it, and say what was found.
pub async fn look_closer(
    scene: &Scene,
    seen: &Seen,
    identifier: &Identifier,
    researcher: Option<&Researcher>,
) -> Result<Known, String> {
    if eyes::is_person(&seen.phrase) {
        return Err("a las personas no las busco en internet".into());
    }
    let crop = eyes::crop_png(&scene.frame, seen.bbox, &eyes::faces(scene))?;
    let identified = identifier.identify(&crop, &seen.phrase).await?;
    let (name, by, mut sources) = match identified {
        Some(found) => (found.name, found.by.to_string(), found.pages),
        None => (seen.phrase.clone(), "iris".to_string(), Vec::new()),
    };
    let mut facts = String::new();
    if by != "iris" {
        if let Some(researcher) = researcher {
            if let Ok(results) = researcher.search(&name).await {
                facts = facts_from(&results);
                sources.extend(results.into_iter().map(|(_, url, _)| url).take(2));
            }
        }
    }
    Ok(Known {
        seen_as: seen.phrase.clone(),
        name,
        by,
        facts,
        sources,
        bbox: seen.bbox,
        at: Some(Instant::now()),
    })
}

/// The object curiosity would look at next: prominent, confident, not a person, and not
/// already in the buffer.
pub fn next_to_look_at<'a>(scene: &'a Scene, buffer: &Buffer) -> Option<&'a Seen> {
    let frame_area = (scene.width.max(1) * scene.height.max(1)) as f32;
    scene
        .seen
        .iter()
        .filter(|item| !eyes::is_person(&item.phrase) && item.score >= 0.45)
        .filter(|item| {
            let area = (item.bbox[2] - item.bbox[0]) * (item.bbox[3] - item.bbox[1]);
            area / frame_area >= 0.02
        })
        .filter(|item| !buffer.covers(item))
        .max_by(|a, b| {
            let weight = |item: &Seen| item.score * (item.bbox[2] - item.bbox[0]) * (item.bbox[3] - item.bbox[1]);
            weight(a).total_cmp(&weight(b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn seen(phrase: &str, score: f32, bbox: [f32; 4]) -> Seen {
        Seen { phrase: phrase.into(), score, bbox }
    }

    fn scene(items: Vec<Seen>) -> Scene {
        Scene { seen: items, width: 100, height: 100, frame: Arc::new(Vec::new()), at: Some(Instant::now()), step: None }
    }

    #[test]
    fn curiosity_skips_people_small_things_and_what_it_already_knows() {
        let scene = scene(vec![
            seen("person", 0.99, [0.0, 0.0, 100.0, 100.0]),
            seen("toy", 0.7, [10.0, 10.0, 40.0, 40.0]),
            seen("pen", 0.9, [0.0, 0.0, 5.0, 5.0]),
        ]);
        let mut buffer = Buffer::default();
        assert_eq!(next_to_look_at(&scene, &buffer).unwrap().phrase, "toy");
        buffer.remember(Known {
            seen_as: "toy".into(),
            name: "Froakie plush".into(),
            by: "web".into(),
            facts: "A Water-type Pokémon.".into(),
            sources: vec![],
            bbox: [12.0, 12.0, 41.0, 41.0],
            at: Some(Instant::now()),
        });
        assert!(next_to_look_at(&scene, &buffer).is_none());
        let told = buffer.describe();
        assert!(told.contains("Froakie plush") && told.contains("no lo digas por decirlo"), "{told}");
    }

    #[test]
    fn the_buffer_keeps_one_entry_per_name_newest_first() {
        let mut buffer = Buffer::default();
        for name in ["a", "b", "A"] {
            buffer.remember(Known {
                seen_as: "toy".into(),
                name: name.into(),
                by: "web".into(),
                facts: String::new(),
                sources: vec![],
                bbox: [0.0; 4],
                at: None,
            });
        }
        let names: Vec<&str> = buffer.entries().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["A", "b"]);
    }

    #[test]
    fn facts_are_short_enough_to_say() {
        let long = "x".repeat(1000);
        let facts = facts_from(&[
            ("t".into(), "u".into(), long.clone()),
            ("t".into(), "u".into(), "second".into()),
            ("t".into(), "u".into(), "third".into()),
        ]);
        assert!(facts.len() < 300 + " / second".len() + 5);
        assert!(!facts.contains("third"));
    }
}
