//! The social state of each domain, and the local expert (LNCM) that keeps it and answers from it.
//!
//! The state lives in the Vault, one sealed entry per domain, graded Restricted: it never
//! leaves the machine and a work domain never sees a personal one. It persists — what to expect
//! of someone is useful precisely because it outlives the messages that showed it — and the
//! owner can forget a person or retract or confirm any expectation.
//!
//! New events are read incrementally behind a cursor. Only conversation sources are read
//! (notifications and the visible WhatsApp chat); IRIS observations are not messages.

use crate::{
    config::LncmBinding,
    store::WandaStore,
    types::{Author, EventSource, Refined, StoredEvent},
    WandaError, MAX_CONTEXT,
};
use archie_cerebra::vault_io::{open_vault, persist_vault, VaultGuard};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, EntryClass, VaultRef};
use archie_wanda_lncm::model::{ComposeOptions, Lncm};
use archie_wanda_lncm::reader::{self, LncmInterpreter, ReadReport, ReaderConfig, Turn, TurnKind};
use archie_wanda_lncm::state::{SocialState, Standing};
use archie_wanda_lncm::{Lang, QuickAnswer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The owner's id in every state.
pub const OWNER_ID: &str = "owner";
const MAX_STATE_BYTES: usize = 4 * 1024 * 1024;
/// Turns a question reads before it is answered (about two of the reader's chunks); a longer
/// backlog is left to the proactive tick, so a question never waits for a whole history.
pub const INLINE_READ_TURNS: usize = 48;
/// Turns one proactive tick reads per domain, so the inference permit is let go between ticks.
pub const TICK_READ_TURNS: usize = 480;
const ACTOR: &str = "wanda-social";

pub struct LncmRuntime {
    base: Lncm,
    adapters: BTreeMap<ContextDomain, Lncm>,
    pub lang: Lang,
    pub gate_threshold: f32,
    pub update_threshold: f32,
    pub owner_label: String,
    pub compose_candidates: usize,
}

impl std::fmt::Debug for LncmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LncmRuntime")
    }
}

impl LncmRuntime {
    /// Opens the checkpoint (every file checked against its manifest) and each domain's adapter.
    pub fn load(binding: &LncmBinding) -> Result<Self, WandaError> {
        let device = archie_wanda_lncm::train::device(&binding.device)
            .map_err(|_| WandaError::Config("the LNCM device is unavailable".into()))?;
        let base = Lncm::load(&binding.dir, &device)
            .map_err(|e| WandaError::Config(format!("the LNCM checkpoint was refused: {e}")))?;
        let mut adapters = BTreeMap::new();
        for (domain, name) in &binding.adapters {
            let model = base
                .with_adapter(name)
                .map_err(|e| WandaError::Config(format!("the LNCM adapter was refused: {e}")))?;
            adapters.insert(domain.clone(), model);
        }
        Ok(Self {
            base,
            adapters,
            lang: binding.lang.unwrap_or(Lang::Es),
            gate_threshold: binding.gate_threshold,
            update_threshold: binding.update_threshold,
            owner_label: binding.owner_label.clone(),
            compose_candidates: binding.compose_candidates.clamp(1, 8),
        })
    }

    pub fn model(&self, domain: &ContextDomain) -> &Lncm {
        self.adapters.get(domain).unwrap_or(&self.base)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub at_ms: i64,
    /// Events read at exactly `at_ms`, so one that arrives later with the same time is not lost.
    pub refs: Vec<String>,
}

fn state_ref(domain: &ContextDomain) -> Result<VaultRef, WandaError> {
    domain
        .reference("wanda/social/state")
        .map_err(|_| WandaError::Storage)
}

fn cursor_ref(domain: &ContextDomain) -> Result<VaultRef, WandaError> {
    domain
        .reference("wanda/social/cursor")
        .map_err(|_| WandaError::Storage)
}

fn disclose_text(
    vault: &mut VaultGuard,
    reference: &VaultRef,
    now_ms: i64,
) -> Result<Option<String>, WandaError> {
    if !vault.contains(reference) {
        return Ok(None);
    }
    let content = vault
        .disclose(ACTOR, reference, PrivacyGrade::Restricted, now_ms)
        .map_err(|_| WandaError::Storage)?;
    Ok(Some(content.text().ok_or(WandaError::Storage)?.to_string()))
}

/// The domain's state and cursor; a fresh state when there is none.
pub fn load(
    vault: &mut VaultGuard,
    domain: &ContextDomain,
    owner_label: &str,
    now_ms: i64,
) -> Result<(SocialState, Cursor), WandaError> {
    let state = match disclose_text(vault, &state_ref(domain)?, now_ms)? {
        Some(text) => SocialState::from_json(&text).map_err(|_| WandaError::Storage)?,
        None => SocialState::new(OWNER_ID, owner_label).map_err(|_| WandaError::Storage)?,
    };
    let cursor = match disclose_text(vault, &cursor_ref(domain)?, now_ms)? {
        Some(text) => serde_json::from_str(&text).map_err(|_| WandaError::Storage)?,
        None => Cursor::default(),
    };
    Ok((state, cursor))
}

/// The domain's state and cursor from memory when this service already holds them; otherwise
/// from the Vault, and then kept. Only [`save`] writes them, and [`remember`] follows every save.
pub fn load_cached(
    store: &WandaStore,
    domain: &ContextDomain,
    owner_label: &str,
    now_ms: i64,
) -> Result<(SocialState, Cursor), WandaError> {
    if let Some(hit) = store
        .social_cache()
        .lock()
        .map_err(|_| WandaError::Storage)?
        .get(domain)
        .cloned()
    {
        return Ok(hit);
    }
    let loaded = {
        let mut vault = open_vault(&store.vault_path()).map_err(|_| WandaError::Storage)?;
        load(&mut vault, domain, owner_label, now_ms)?
    };
    remember(store, domain, &loaded.0, &loaded.1)?;
    Ok(loaded)
}

/// Keeps what was just persisted as the domain's state in memory.
fn remember(
    store: &WandaStore,
    domain: &ContextDomain,
    state: &SocialState,
    cursor: &Cursor,
) -> Result<(), WandaError> {
    store
        .social_cache()
        .lock()
        .map_err(|_| WandaError::Storage)?
        .insert(domain.clone(), (state.clone(), cursor.clone()));
    Ok(())
}

pub fn save(
    vault: &mut VaultGuard,
    domain: &ContextDomain,
    state: &SocialState,
    cursor: &Cursor,
    now_ms: i64,
) -> Result<(), WandaError> {
    let state_text = serde_json::to_string(state).map_err(|_| WandaError::Storage)?;
    if state_text.len() > MAX_STATE_BYTES {
        return Err(WandaError::TooLarge);
    }
    let cursor_text = serde_json::to_string(cursor).map_err(|_| WandaError::Storage)?;
    for (reference, text) in [
        (state_ref(domain)?, state_text),
        (cursor_ref(domain)?, cursor_text),
    ] {
        vault
            .store(
                ACTOR,
                reference,
                EntryClass::PrivateMemory,
                PrivacyGrade::Restricted,
                &text,
                now_ms,
                None,
            )
            .map_err(|_| WandaError::Storage)?;
    }
    Ok(())
}

/// Conversation events not yet read, as turns numbered after the state's last, in time order:
/// at most `limit`, the earliest observed first, so the cursor never passes an unread event.
pub fn turns_from(
    events: &[StoredEvent],
    state: &SocialState,
    cursor: &Cursor,
    limit: usize,
) -> (Vec<Turn>, Cursor) {
    let mut fresh: Vec<&StoredEvent> = events
        .iter()
        .filter(|e| {
            matches!(
                e.event.source,
                EventSource::AndroidNotification | EventSource::WhatsappVisibleDom
            )
        })
        .filter(|e| e.event.author != Author::System)
        .filter(|e| {
            e.event.observed_at_ms > cursor.at_ms
                || (e.event.observed_at_ms == cursor.at_ms
                    && !cursor.refs.contains(&e.evidence_ref.to_string()))
        })
        .collect();
    if fresh.len() > limit {
        // The cursor advances by observation time: keep the earliest observed.
        fresh.sort_by(|a, b| {
            a.event
                .observed_at_ms
                .cmp(&b.event.observed_at_ms)
                .then_with(|| a.evidence_ref.to_string().cmp(&b.evidence_ref.to_string()))
        });
        fresh.truncate(limit);
    }
    fresh.sort_by(|a, b| {
        let at = |e: &StoredEvent| e.event.sent_at_ms.unwrap_or(e.event.observed_at_ms);
        at(a)
            .cmp(&at(b))
            .then_with(|| a.evidence_ref.to_string().cmp(&b.evidence_ref.to_string()))
    });
    let mut next = cursor.clone();
    let mut turns = Vec::with_capacity(fresh.len());
    for (i, e) in fresh.into_iter().enumerate() {
        let observed = e.event.observed_at_ms;
        if observed > next.at_ms {
            next = Cursor {
                at_ms: observed,
                refs: Vec::new(),
            };
        }
        if observed == next.at_ms {
            next.refs.push(e.evidence_ref.to_string());
        }
        let (speaker, label) = match e.event.author {
            Author::Owner => (OWNER_ID.to_string(), None),
            _ => (e.event.sender.clone(), Some(e.event.sender.clone())),
        };
        let chat =
            crate::chat::routing_key(&e.event.chat_id).unwrap_or_else(|| e.event.chat_id.clone());
        turns.push(Turn {
            turn: state.last_turn + 1 + i as u64,
            at_ms: e.event.sent_at_ms.unwrap_or(observed).max(0),
            chat,
            kind: TurnKind::Message {
                speaker,
                text: e.event.text.clone(),
            },
            evidence: e.evidence_ref.to_string(),
            label,
        });
    }
    (turns, next)
}

/// Reads at most `max_turns` of the domain's new conversation events into its state; the rest
/// waits for the next call. `None` without an expert. Blocking: call from blocking work,
/// under the service's inference permit.
pub fn read_new(
    store: &WandaStore,
    domain: &ContextDomain,
    now_ms: i64,
    max_turns: usize,
) -> Result<Option<ReadReport>, WandaError> {
    let Some(lncm) = store.lncm() else {
        return Ok(None);
    };
    let _writing = store
        .social_write()
        .lock()
        .map_err(|_| WandaError::Storage)?;
    let events = store.events(domain, MAX_CONTEXT, now_ms)?;
    let (mut state, cursor) = load_cached(store, domain, &lncm.owner_label, now_ms)?;
    let (turns, next) = turns_from(&events, &state, &cursor, max_turns);
    if turns.is_empty() {
        return Ok(Some(ReadReport::default()));
    }
    // The model runs with no Vault lock held.
    let interpreter = LncmInterpreter {
        model: lncm.model(domain),
        lang: lncm.lang,
        threshold: lncm.update_threshold,
    };
    let report = reader::read(
        &mut state,
        &turns,
        &interpreter,
        &ReaderConfig {
            lang: lncm.lang,
            ..Default::default()
        },
    )
    .map_err(|_| WandaError::Storage)?;
    let dir = store.vault_path();
    let mut vault = open_vault(&dir).map_err(|_| WandaError::Storage)?;
    save(&mut vault, domain, &state, &next, now_ms)?;
    persist_vault(&dir, &vault).map_err(|_| WandaError::Storage)?;
    remember(store, domain, &state, &next)?;
    Ok(Some(report))
}

/// Who a screen's or camera's text is attributed to in a domain's state.
pub const SCENE_ID: &str = "scene";
const SCENE_LABEL: &str = "Pantalla";
/// Lines one call reads: a `scene.text` delta's own limit.
pub const MAX_SCENE_LINES: usize = 256;

/// Text IRIS saw (`scene.text` lines), read into the domain's state the way conversation
/// events are (stage A of the bridge, plan §7.2): one chat per focus, every line said by
/// [`SCENE_ID`], cited as `iris:<focus>:<track>`. The state changes in place, so what was
/// seen never grows a transcript to compact; the conversation cursor does not move, these
/// are not Vault events. `None` without an expert. Blocking, under the inference permit.
pub fn read_scene(
    store: &WandaStore,
    domain: &ContextDomain,
    focus: &str,
    lines: &[archie_slink::links::SceneTextLine],
    now_ms: i64,
) -> Result<Option<ReadReport>, WandaError> {
    if lines.len() > MAX_SCENE_LINES {
        return Err(WandaError::TooLarge);
    }
    let Some(lncm) = store.lncm() else {
        return Ok(None);
    };
    if lines.is_empty() {
        return Ok(Some(ReadReport::default()));
    }
    let _writing = store
        .social_write()
        .lock()
        .map_err(|_| WandaError::Storage)?;
    let (mut state, cursor) = load_cached(store, domain, &lncm.owner_label, now_ms)?;
    let turns: Vec<Turn> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| Turn {
            turn: state.last_turn + 1 + i as u64,
            at_ms: i64::try_from(line.observed_at_ms).unwrap_or(now_ms),
            chat: format!("scene:{focus}"),
            kind: TurnKind::Message {
                speaker: SCENE_ID.to_string(),
                text: line.text.clone(),
            },
            evidence: format!("iris:{focus}:{}", line.track),
            label: Some(SCENE_LABEL.to_string()),
        })
        .collect();
    let interpreter = LncmInterpreter {
        model: lncm.model(domain),
        lang: lncm.lang,
        threshold: lncm.update_threshold,
    };
    let report = reader::read(
        &mut state,
        &turns,
        &interpreter,
        &ReaderConfig {
            lang: lncm.lang,
            ..Default::default()
        },
    )
    .map_err(|_| WandaError::Storage)?;
    let dir = store.vault_path();
    let mut vault = open_vault(&dir).map_err(|_| WandaError::Storage)?;
    save(&mut vault, domain, &state, &cursor, now_ms)?;
    persist_vault(&dir, &vault).map_err(|_| WandaError::Storage)?;
    remember(store, domain, &state, &cursor)?;
    Ok(Some(report))
}

/// Where `archie_slink::text::Batcher` hands seen text: one domain's state in this store.
pub struct SceneSink {
    pub store: WandaStore,
    pub domain: ContextDomain,
}

impl archie_slink::text::StateUpdate for SceneSink {
    fn update(
        &mut self,
        focus: &str,
        lines: &[archie_slink::links::SceneTextLine],
    ) -> Result<(), String> {
        read_scene(
            &self.store,
            &self.domain,
            focus,
            lines,
            crate::server::now_ms(),
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
}

/// The state as the expert reads it for a question, people named in the question first, and
/// the names it may use.
pub fn context_for(state: &SocialState, question: &str, lang: Lang) -> (String, Vec<String>) {
    let words: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let focus: Vec<&str> = state
        .people
        .values()
        .filter(|p| {
            p.label
                .split_whitespace()
                .next()
                .is_some_and(|first| words.iter().any(|w| *w == first.to_lowercase()))
        })
        .map(|p| p.id.as_str())
        .collect();
    let roster = state.people.values().map(|p| p.label.clone()).collect();
    (state.render(&focus, 2048, lang), roster)
}

/// State 1 (an immediate closed answer) and state 2 (the refined long answer).
pub type States = (Option<QuickAnswer>, Option<Refined>);

/// States 1 and 2 for a question, or `None` when there is no expert or the question is not
/// social. Blocking.
pub fn answer(
    store: &WandaStore,
    domain: &ContextDomain,
    question: &str,
    draft: Option<&str>,
    now_ms: i64,
) -> Result<Option<States>, WandaError> {
    let Some(lncm) = store.lncm() else {
        return Ok(None);
    };
    let model = lncm.model(domain);
    let gate = model
        .gate(lncm.lang, question)
        .map_err(|_| WandaError::NoModel)?;
    if gate.social < lncm.gate_threshold {
        return Ok(None);
    }
    let (state, _) = load_cached(store, domain, &lncm.owner_label, now_ms)?;
    let (text, roster) = context_for(&state, question, lncm.lang);
    // What the bookkeeping answers by itself is not left to the decision head.
    let quick = match archie_wanda_lncm::exact::resolve(&state, question) {
        Some(exact) => Some(exact.quick(lncm.lang)),
        None => model
            .quick_after_gate(&gate, lncm.lang, question, Some(&text), draft)
            .ok()
            .flatten(),
    };
    // Several candidates come from one batch; the model keeps one that passes the checks.
    let options = ComposeOptions {
        candidates: lncm.compose_candidates,
        sampling: archie_wanda_lncm::generator::Sampling {
            temperature: 0.7,
            top_k: 20,
        },
        seed: u64::try_from(now_ms).unwrap_or(0),
    };
    let refined = model
        .compose(lncm.lang, question, Some(&text), draft, &roster, options)
        .ok()
        .filter(|c| !c.answer.is_empty())
        .map(|c| Refined {
            plan: c.plan,
            answer: c.answer,
            unverified_names: c.unverified_names,
            seconds: c.seconds,
            verified: c.verified,
            corrected_values: c.corrected_values,
            problems: c.problems,
        });
    Ok(Some((quick, refined)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectationView {
    pub key: String,
    pub statement: String,
    pub confidence: f32,
    pub evidence: f32,
    pub standing: Standing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonSummary {
    pub id: String,
    pub label: String,
    pub expectations: Vec<ExpectationView>,
    pub affect: Option<(String, u8)>,
    pub messages: u32,
    pub reply_latency_ms: Option<f64>,
    /// Topic → the value this person last saw.
    pub seen: BTreeMap<String, String>,
}

/// What to expect of each person in a domain, strongest first.
pub fn people(
    store: &WandaStore,
    domain: &ContextDomain,
    now_ms: i64,
) -> Result<Vec<PersonSummary>, WandaError> {
    store.require_domain(domain)?;
    let owner_label = store
        .lncm()
        .map_or("Yo".to_string(), |l| l.owner_label.clone());
    let (state, _) = load_cached(store, domain, &owner_label, now_ms)?;
    Ok(state
        .people
        .values()
        .map(|p| PersonSummary {
            id: p.id.clone(),
            label: p.label.clone(),
            expectations: p
                .what_to_expect()
                .into_iter()
                .map(|e| ExpectationView {
                    key: e.key.clone(),
                    statement: e.statement.clone(),
                    confidence: e.confidence(),
                    evidence: e.weight(),
                    standing: e.standing,
                })
                .collect(),
            affect: p.affect.as_ref().map(|a| (a.label.clone(), a.intensity)),
            messages: p.behaviour.messages,
            reply_latency_ms: p.behaviour.reply_latency_ms,
            seen: p
                .seen
                .iter()
                .map(|(t, s)| (t.clone(), s.value.clone()))
                .collect(),
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditAction {
    /// The owner agrees with an inferred expectation (it stays a hypothesis).
    Confirm,
    /// The owner says an expectation is wrong: it is removed.
    Retract,
    /// Everything about the person is removed.
    Forget,
}

/// Applies an owner's edit. Returns whether anything changed. Blocking.
pub fn edit(
    store: &WandaStore,
    domain: &ContextDomain,
    person: &str,
    key: Option<&str>,
    action: EditAction,
    now_ms: i64,
) -> Result<bool, WandaError> {
    store.require_domain(domain)?;
    if person.trim().is_empty()
        || person.len() > 128
        || key.is_some_and(|k| k.is_empty() || k.len() > 64)
    {
        return Err(WandaError::InvalidEvent("invalid_edit"));
    }
    let owner_label = store
        .lncm()
        .map_or("Yo".to_string(), |l| l.owner_label.clone());
    let _writing = store
        .social_write()
        .lock()
        .map_err(|_| WandaError::Storage)?;
    let dir = store.vault_path();
    let mut vault = open_vault(&dir).map_err(|_| WandaError::Storage)?;
    let (mut state, cursor) = load(&mut vault, domain, &owner_label, now_ms)?;
    let changed = match (action, key) {
        (EditAction::Confirm, Some(k)) => state.confirm(person, k),
        (EditAction::Retract, Some(k)) => state.retract(person, k),
        (EditAction::Forget, None) => state.forget(person),
        _ => return Err(WandaError::InvalidEvent("invalid_edit")),
    };
    if changed {
        save(&mut vault, domain, &state, &cursor, now_ms)?;
        persist_vault(&dir, &vault).map_err(|_| WandaError::Storage)?;
    }
    remember(store, domain, &state, &cursor)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Actor, Coverage, WandaEvent};

    fn event(id: &str, author: Author, sender: &str, text: &str, observed: i64) -> StoredEvent {
        let domain = ContextDomain::personal("owner").unwrap();
        StoredEvent {
            evidence_ref: domain.reference(&format!("wanda/events/{id}")).unwrap(),
            domain,
            event: WandaEvent {
                schema_version: 1,
                event_id: id.into(),
                device_id: "phone".into(),
                actor: Actor::ConfirmedOwner,
                author,
                source: EventSource::WhatsappVisibleDom,
                chat_id: "120363042000000000@g.us".into(),
                sender: sender.into(),
                source_message_id: None,
                sent_at_ms: None,
                observed_at_ms: observed,
                text: text.into(),
                media_omitted: false,
                coverage: Coverage::VisibleChat,
                monitor_id: None,
                window_id: None,
            },
        }
    }

    #[test]
    fn events_become_ordered_turns_behind_a_cursor() {
        let state = SocialState::new(OWNER_ID, "Yo").unwrap();
        let events = vec![
            event("b", Author::Counterparty, "Pedro", "llego tarde", 2000),
            event("a", Author::Owner, "Yo", "¿vamos?", 1000),
            event("c", Author::Counterparty, "Ana", "voy", 2000),
        ];
        let (turns, cursor) = turns_from(&events, &state, &Cursor::default(), usize::MAX);
        assert_eq!(turns.len(), 3);
        assert!(matches!(&turns[0].kind, TurnKind::Message { speaker, .. } if speaker == OWNER_ID));
        assert_eq!(turns[0].turn, 1);
        assert_eq!(turns[0].chat, "120363042000000000@g.us");
        assert_eq!(cursor.at_ms, 2000);
        assert_eq!(cursor.refs.len(), 2);
        // Nothing is read twice; an event at the cursor's time that was not read is.
        let (again, _) = turns_from(&events, &state, &cursor, usize::MAX);
        assert!(again.is_empty());
        let mut more = events.clone();
        more.push(event("d", Author::Counterparty, "Luis", "ok", 2000));
        let (late, _) = turns_from(&more, &state, &cursor, usize::MAX);
        assert_eq!(late.len(), 1);
    }

    #[test]
    fn a_bounded_read_takes_the_earliest_and_loses_nothing() {
        let state = SocialState::new(OWNER_ID, "Yo").unwrap();
        // Sent order differs from observed order: the bound follows observation time.
        let mut late_sent = event("x", Author::Counterparty, "Ana", "uno", 1000);
        late_sent.event.sent_at_ms = Some(9000);
        let events = vec![
            late_sent,
            event("y", Author::Counterparty, "Pedro", "dos", 2000),
            event("z", Author::Counterparty, "Luis", "tres", 3000),
            event("w", Author::Counterparty, "Luis", "cuatro", 3000),
        ];
        let (first, cursor) = turns_from(&events, &state, &Cursor::default(), 3);
        assert_eq!(first.len(), 3);
        assert_eq!(cursor.at_ms, 3000);
        assert_eq!(
            cursor.refs.len(),
            1,
            "one of the two events at 3000 is still unread"
        );
        let (rest, cursor) = turns_from(&events, &state, &cursor, 3);
        assert_eq!(rest.len(), 1);
        let (none, _) = turns_from(&events, &state, &cursor, 3);
        assert!(none.is_empty());
        let mut seen: Vec<String> = first
            .iter()
            .chain(&rest)
            .map(|t| t.evidence.clone())
            .collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 4, "every event read exactly once");
    }

    #[test]
    fn context_puts_named_people_first() {
        let mut state = SocialState::new(OWNER_ID, "Yo").unwrap();
        state.ensure_person("Pedro", "Pedro", 1);
        state.ensure_person("Ana María", "Ana María", 2);
        let (text, roster) = context_for(&state, "¿Qué espero de ana si le pido algo?", Lang::Es);
        assert!(text.starts_with("Personas: Ana María,"));
        assert_eq!(roster.len(), 3);
    }
}
