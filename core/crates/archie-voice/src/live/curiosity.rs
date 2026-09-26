//! What Archie knows about what it has seen, and has not said.
//!
//! IRIS names things by category — "toy", "car", "bottle". Curiosity looks closer at the
//! prominent object in front of the camera, finds out what exactly it is, reads a little
//! about it, and keeps that in a buffer. The buffer is told to the model as context, marked
//! as known and unsaid: Archie brings it up when asked, or when it answers what was asked.
//! It never volunteers it into silence; the floor decides whether Archie speaks at all.
//!
//! Finding out what an object is goes to a vision model on Nebius Token Factory, on the
//! same key as the conversation (`NEBIUS_TOKEN_FACTORY_KEY`): Kimi K3, then MiniCPM-V 4.5
//! if Kimi cannot tell (`ARCHIE_LIVE_VLM` overrides the list). It names the thing and gives
//! a fact or two, marked as the model's; with `TAVILY_API_KEY`, a search reads more. With
//! no key, IRIS's own category is all there is, said as a category.
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
                if let Some(at) = entry.at {
                    line.push_str(&format!(
                        " (lo miraste hace {}; puede que ya no esté)",
                        crate::live::memory::ago(at.elapsed())
                    ));
                }
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
    /// What the vision model itself knows of it, which may be wrong.
    pub facts: String,
    pub pages: Vec<String>,
    /// The model that answered.
    pub model: String,
}

/// Finds out what an object is from its crop, with a vision model on Nebius Token Factory
/// — the same account and key the conversation uses.
pub struct Identifier {
    http: reqwest::Client,
    key: Option<Zeroizing<String>>,
    /// Tried in order: the first that names the object answers.
    models: Vec<String>,
}

/// Kimi K3 knows characters and products best of the vision models on Nebius, and reasons
/// before it answers; MiniCPM-V 4.5 answers at once, and is asked when Kimi will not say.
/// Measured on 2026-09-26: all five of Nebius's vision models named the Eiffel Tower, and
/// the three that reason returned nothing when held to 40 tokens.
pub const DEFAULT_VISION_MODELS: &[&str] = &["moonshotai/Kimi-K3", "openbmb/MiniCPM-V-4_5"];

/// Room for a reasoning model to think before it answers.
const VISION_MAX_TOKENS: u32 = 2048;

impl Identifier {
    /// Reads the key from `NEBIUS_TOKEN_FACTORY_KEY`, and the models from `ARCHIE_LIVE_VLM`
    /// (comma-separated, in order) or [`DEFAULT_VISION_MODELS`].
    pub fn from_environment() -> Result<Self, String> {
        let key = std::env::var("NEBIUS_TOKEN_FACTORY_KEY")
            .ok()
            .map(|value| Zeroizing::new(value.trim().to_string()))
            .filter(|value| !value.is_empty());
        let models = match std::env::var("ARCHIE_LIVE_VLM") {
            Ok(list) if !list.trim().is_empty() => list
                .split(',')
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty())
                .collect(),
            _ => DEFAULT_VISION_MODELS.iter().map(|model| model.to_string()).collect(),
        };
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .map_err(|e| e.to_string())?,
            key,
            models,
        })
    }

    /// Which ways of finding out are configured, for the page and the log.
    pub fn means(&self) -> Vec<String> {
        match &self.key {
            Some(_) => self.models.iter().map(|model| format!("nebius:{model}")).collect(),
            None => Vec::new(),
        }
    }

    pub async fn identify(&self, png: &[u8], category: &str) -> Result<Option<Identified>, String> {
        let Some(key) = &self.key else {
            return Ok(None);
        };
        let mut last_error = None;
        for model in &self.models {
            match self.vision_model(key, model, png, category).await {
                Ok(Some(found)) => return Ok(Some(found)),
                Ok(None) => {}
                Err(error) => last_error = Some(error),
            }
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
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
            "max_tokens": VISION_MAX_TOKENS,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": vision_question(category)},
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
            .map_err(|e| format!("{model}: {e}"))?
            .error_for_status()
            .map_err(|e| format!("{model}: {}", e.status().map_or("failed".into(), |s| s.to_string())))?
            .json()
            .await
            .map_err(|e| format!("{model} answer: {e}"))?;
        let text = answer["choices"][0]["message"]["content"].as_str().unwrap_or("");
        Ok(parse_identification(text).map(|(name, facts)| Identified {
            name,
            by: "vision-model",
            facts,
            pages: Vec::new(),
            model: model.to_string(),
        }))
    }
}

fn vision_question(category: &str) -> String {
    format!(
        "A detector labelled this object \"{category}\". What exactly is it? Reply with one \
         line of JSON and nothing else: {{\"name\": \"its specific name — brand and model, \
         character, species, product or landmark — in at most eight words\", \"facts\": \"one \
         or two short, well-known facts about it, or an empty string\"}}. If you cannot tell \
         what it is, reply {{\"name\": \"unknown\", \"facts\": \"\"}}. Never name a person."
    )
}

/// The name and facts from a vision model's reply: its JSON, or failing that, its first
/// line as the name. `None` when it could not tell.
fn parse_identification(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let json = text
        .find('{')
        .zip(text.rfind('}'))
        .and_then(|(start, end)| serde_json::from_str::<serde_json::Value>(&text[start..=end]).ok());
    let (name, facts) = match json {
        Some(value) => (
            value["name"].as_str().unwrap_or("").to_string(),
            value["facts"].as_str().unwrap_or("").trim().to_string(),
        ),
        None => (text.lines().next().unwrap_or("").to_string(), String::new()),
    };
    let name = name.trim().trim_matches(|c: char| c == '.' || c == '"').trim().to_string();
    (!name.is_empty() && !name.eq_ignore_ascii_case("unknown")).then_some((name, facts))
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
    let (name, by, mut facts, mut sources) = match identified {
        Some(found) => (found.name, found.by.to_string(), found.facts, vec![format!("nebius:{}", found.model)]),
        None => (seen.phrase.clone(), "iris".to_string(), String::new(), Vec::new()),
    };
    if by != "iris" {
        if let Some(researcher) = researcher {
            if let Ok(results) = researcher.search(&name).await {
                let read = facts_from(&results);
                if !read.is_empty() {
                    // What the web says goes first; the model's own recollection after it.
                    facts = if facts.is_empty() { read } else { format!("{read} (Y según el modelo de visión: {facts})") };
                }
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

    #[test]
    fn a_vision_answer_is_read_as_json_or_as_its_first_line() {
        assert_eq!(
            parse_identification("```json\n{\"name\": \"Froakie plush\", \"facts\": \"A Water-type Pokémon.\"}\n```"),
            Some(("Froakie plush".into(), "A Water-type Pokémon.".into()))
        );
        assert_eq!(parse_identification("Eiffel Tower."), Some(("Eiffel Tower".into(), String::new())));
        assert_eq!(parse_identification("{\"name\": \"unknown\", \"facts\": \"\"}"), None);
        assert_eq!(parse_identification(""), None);
        assert!(vision_question("toy").contains("Never name a person"));
    }

    fn seen(phrase: &str, score: f32, bbox: [f32; 4]) -> Seen {
        Seen { phrase: phrase.into(), score, bbox, embedding: None }
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
