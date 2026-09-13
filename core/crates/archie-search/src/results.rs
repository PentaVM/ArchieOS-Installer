//! What a search returns, admitted as third-party data.
//!
//! Every field here was written by somebody on the internet. It may contain
//! instructions, and it may contain bytes chosen to do something to whatever displays
//! it. So a result is bounded in every dimension, stripped of control characters and of
//! the bidirectional overrides that make a terminal show one thing and hold another, and
//! kept only if it points at an ordinary `http` or `https` page.
//!
//! It is still data. Nothing here makes result text safe to put into a prompt as
//! anything but fenced, cited evidence; `reference` exists so a later stage can cite a
//! result without quoting it.

use reqwest::Url;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_SNIPPET_CHARS: usize = 2_000;
pub const MAX_URL_BYTES: usize = 2_048;
pub const MAX_PUBLISHED_CHARS: usize = 64;

/// The results of one search.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResults {
    pub provider_id: String,
    /// In the provider's ranking order.
    pub results: Vec<WebResult>,
    /// Entries returned but not admitted: unusable URL, nothing to show, a duplicate,
    /// or more than were asked for. Counted, never shown.
    pub discarded: usize,
    /// Admitted entries with at least one field cut to its bound.
    pub truncated: usize,
    /// How long the provider says it took.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_time_ms: Option<u32>,
}

/// One page the provider found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebResult {
    /// `web:` and the first 32 hex digits of the URL's SHA-256.
    pub reference: String,
    pub url: String,
    pub title: String,
    pub snippet: String,
    /// The provider's relevance score, 0..=1000.
    pub score_permille: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}

impl WebResult {
    /// The citable reference for a URL.
    pub fn reference_for(url: &str) -> String {
        let digest = Sha256::digest(url.as_bytes());
        format!("web:{}", &hex::encode(digest)[..32])
    }
}

/// A result as the provider sent it. Every field optional: a missing field makes one
/// entry unusable, not the whole response.
#[derive(Debug, Clone, Default)]
pub(crate) struct RawResult {
    pub title: Option<String>,
    pub url: Option<String>,
    pub content: Option<String>,
    pub score: Option<f64>,
    pub published_date: Option<String>,
}

/// Admit at most `limit` results from what the provider returned.
pub(crate) fn admit(
    provider_id: &str,
    raw: Vec<RawResult>,
    limit: usize,
    response_time_ms: Option<u32>,
) -> SearchResults {
    let mut results: Vec<WebResult> = Vec::new();
    let mut discarded = 0;
    let mut truncated = 0;

    for entry in raw {
        if results.len() == limit {
            discarded += 1;
            continue;
        }
        let Some(url) = entry.url.as_deref().and_then(web_url) else {
            discarded += 1;
            continue;
        };
        let reference = WebResult::reference_for(&url);
        if results.iter().any(|r| r.reference == reference) {
            discarded += 1;
            continue;
        }
        let (title, cut_title) = clean(entry.title.as_deref(), MAX_TITLE_CHARS, false);
        let (snippet, cut_snippet) = clean(entry.content.as_deref(), MAX_SNIPPET_CHARS, true);
        if title.is_empty() && snippet.is_empty() {
            discarded += 1;
            continue;
        }
        let score_permille = match entry.score {
            Some(score) if score.is_finite() => (score.clamp(0.0, 1.0) * 1000.0).round() as u16,
            Some(_) => {
                discarded += 1;
                continue;
            }
            None => 0,
        };
        let (published, cut_published) =
            clean(entry.published_date.as_deref(), MAX_PUBLISHED_CHARS, false);
        if cut_title || cut_snippet || cut_published {
            truncated += 1;
        }
        results.push(WebResult {
            reference,
            url,
            title,
            snippet,
            score_permille,
            published: (!published.is_empty()).then_some(published),
        });
    }

    SearchResults {
        provider_id: provider_id.to_string(),
        results,
        discarded,
        truncated,
        response_time_ms,
    }
}

/// An ordinary web page address, normalised, or `None`.
fn web_url(raw: &str) -> Option<String> {
    if raw.len() > MAX_URL_BYTES {
        return None;
    }
    let url = Url::parse(raw.trim()).ok()?;
    let ordinary = matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none();
    ordinary.then(|| url.to_string())
}

/// Bound `text` to `max_chars`, replacing control and bidirectional-formatting
/// characters with spaces. Newlines survive only where `keep_newlines` says so. Returns
/// whether anything was cut.
fn clean(text: Option<&str>, max_chars: usize, keep_newlines: bool) -> (String, bool) {
    let Some(text) = text else {
        return (String::new(), false);
    };
    let mut out = String::with_capacity(text.len().min(max_chars * 4));
    let mut cut = false;
    for (count, ch) in text.chars().enumerate() {
        if count == max_chars {
            cut = true;
            break;
        }
        out.push(if ch == '\n' && keep_newlines {
            '\n'
        } else if ch.is_control() || is_bidi_control(ch) {
            ' '
        } else {
            ch
        });
    }
    (out.trim().to_string(), cut)
}

/// Unicode's explicit directional embeddings, overrides and isolates, plus the marks.
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(url: &str, title: &str, score: f64) -> RawResult {
        RawResult {
            title: Some(title.into()),
            url: Some(url.into()),
            content: Some("a snippet".into()),
            score: Some(score),
            published_date: None,
        }
    }

    #[test]
    fn only_ordinary_web_addresses_are_admitted() {
        assert!(web_url("https://arxiv.org/abs/1").is_some());
        assert!(web_url("http://example.com").is_some());
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            // Each half of the userinfo on its own, so neither check hides the other.
            "https://user@example.com/",
            "https://:pw@example.com/",
            "not a url",
        ] {
            assert!(web_url(bad).is_none(), "{bad}");
        }
        assert!(web_url(&format!("https://e.com/{}", "a".repeat(MAX_URL_BYTES))).is_none());
    }

    #[test]
    fn terminal_control_and_bidi_overrides_are_removed() {
        let (title, cut) = clean(Some("safe\u{1b}[2Jtitle\u{202E}txt.exe"), 300, false);
        assert!(!cut);
        assert!(!title.contains('\u{1b}'));
        assert!(!title.contains('\u{202E}'));
        assert_eq!(title, "safe [2Jtitle txt.exe");
        let (snippet, _) = clean(Some("line one\nline two\r\n"), 300, true);
        assert_eq!(snippet, "line one\nline two");
        let (title, _) = clean(Some("line one\nline two"), 300, false);
        assert_eq!(title, "line one line two");
    }

    #[test]
    fn text_is_cut_on_character_boundaries() {
        let (text, cut) = clean(Some(&"ñ".repeat(10)), 4, false);
        assert!(cut);
        assert_eq!(text, "ññññ");
    }

    #[test]
    fn duplicates_unusable_entries_and_extras_are_counted_not_kept() {
        let entries = vec![
            raw("https://a.example/", "A", 0.9),
            raw("https://a.example/", "A again", 0.8),
            raw("javascript:void(0)", "bad", 0.7),
            RawResult {
                url: Some("https://empty.example/".into()),
                ..RawResult::default()
            },
            raw("https://b.example/", "B", 1.7),
            raw("https://c.example/", "C", 0.5),
        ];
        let results = admit("tavily", entries, 2, Some(120));
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.discarded, 4);
        assert_eq!(results.results[0].title, "A");
        assert_eq!(
            results.results[1].score_permille, 1000,
            "clamped, not refused"
        );
        assert_eq!(results.response_time_ms, Some(120));
    }

    #[test]
    fn a_reference_is_stable_and_does_not_quote_the_url() {
        let reference = WebResult::reference_for("https://arxiv.org/abs/1");
        assert_eq!(
            reference,
            WebResult::reference_for("https://arxiv.org/abs/1")
        );
        assert!(reference.starts_with("web:"));
        assert_eq!(reference.len(), 4 + 32);
        assert!(!reference.contains("arxiv"));
    }
}
