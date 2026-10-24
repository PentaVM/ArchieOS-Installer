//! The cognitive plane, backed by Nemotron.
//!
//! This is the half of the conversation that decides what to say. It hears the transcript
//! the speech plane produced, asks a Nemotron model on Nebius for an answer, and hands back
//! three things as they arrive: pieces of the answer that a voice can say, tools the model
//! proposes, and the end of the turn.
//!
//! It keeps to the speech plane's contract in the ways that matter:
//!
//! - The answer is released in whole sentences or clauses, never in the sub-word fragments
//!   the model streams, and each piece carries its place in the turn without gaps.
//! - The floor's budget is honoured at a sentence boundary: granted one sentence, the voice
//!   gets one complete sentence and nothing after it.
//! - A tool the model asks for is a proposal and only that. It is passed on with the
//!   model's arguments untouched; whether it happens is Cerebra's and Policy's decision.
//! - A turn that is stopped emits nothing further — not even that it completed, because it
//!   did not.
//! - What the model reasons privately is never spoken; the transport drops it before it
//!   could reach here.
//!
//! Facts the model cannot know — the time, the date, what is on the screen — are given to
//! it by whoever asks, through [`NemotronCognitive::set_context`]. Asked the time in Tokyo
//! with nothing to go on, the fastest Nemotron answered "son las 14:30" with complete
//! confidence; the persona tells it not to guess, and the context gives it no reason to.

use crate::segment::{Segmenter, Spoken};
use archie_model_runtime::routed::{ChatDisclosure, Routed, RoutedError};
use archie_model_runtime::{
    provider_model_id, InferenceClient, ProviderEndpoint, RuntimeError, StreamEvent,
    StreamUnveiler, ToolCall, ToolChatMessage, ToolChatRequest, ToolDefinition,
};
use archie_speech::{
    validate_request, CancelReason, CognitiveError, CognitiveEvent, CognitiveModelAdapter,
    CognitiveRequest, ResponseGate, ResponseSegment, SpeechBudget, SpeechToolIntentCandidate,
    SpokenContentSource,
};
use archie_types::{ModelDescriptor, ModelRole, PrivacyGrade};
use archie_vault::Veil;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Something the transport reports while a turn is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Event(StreamEvent),
    /// The stream could not be read to its end.
    Failed(String),
}

/// A running stream: where its updates arrive, and how to stop it.
pub struct TurnStream {
    pub updates: Receiver<Update>,
    pub stop: Box<dyn FnMut() + Send>,
}

/// Starts a stream for a request. The real one asks Nebius; a test replays a recording.
pub trait TurnSource {
    fn start(&mut self, request: ToolChatRequest) -> TurnStream;
}

/// Asks whichever model the router chooses, preferring local models and veiling text for cloud.
pub struct RoutedSource {
    routed: Arc<Routed>,
    privacy: PrivacyGrade,
    role: ModelRole,
    veil: Arc<Veil>,
    timeout: Duration,
    runtime: tokio::runtime::Handle,
}

impl RoutedSource {
    pub fn new(
        routed: Arc<Routed>,
        privacy: PrivacyGrade,
        role: ModelRole,
        veil: Arc<Veil>,
        timeout: Duration,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            routed,
            privacy,
            role,
            veil,
            timeout,
            runtime,
        }
    }
}

impl TurnSource for RoutedSource {
    fn start(&mut self, request: ToolChatRequest) -> TurnStream {
        let (send, updates) = mpsc::channel();
        let routed = self.routed.clone();
        let veil = self.veil.clone();
        let privacy = self.privacy;
        let role = self.role;
        let timeout = self.timeout;

        let task = self.runtime.spawn(async move {
            // A local server that has stopped is offline here, so the turn goes veiled to
            // the cloud when its grade allows, and is refused aloud when it does not.
            routed.refresh_liveness().await;
            match routed.disclose_chat(&request, privacy, role, &veil) {
                Err(RoutedError::Withheld(reason)) => {
                    let refusal = match privacy {
                        PrivacyGrade::Secret => {
                            "No puedo responder a esto porque los datos son secretos y no deben comunicarse a ningún modelo."
                        }
                        _ => {
                            "No puedo responder a esto porque los datos son confidenciales y no hay un modelo local en este equipo disponible."
                        }
                    };
                    let _ = send.send(Update::Event(StreamEvent::Text(refusal.to_string())));
                    let _ = send.send(Update::Event(StreamEvent::Finished {
                        reason: Some(format!("withheld: {reason}")),
                    }));
                }
                Err(RoutedError::NoModel(reason)) => {
                    let _ = send.send(Update::Failed(format!("no model may answer: {reason}")));
                }
                Err(err) => {
                    let _ = send.send(Update::Failed(err.to_string()));
                }
                Ok(disclosure) => {
                    let (models, req, unveil_opt) = match disclosure {
                        ChatDisclosure::Kept { models, request } => (models, request, None),
                        ChatDisclosure::Veiled {
                            models,
                            request,
                            unveil,
                        } => (models, request, Some(unveil)),
                    };

                    let started = Instant::now();
                    let mut last_error = String::from("no model answered");
                    let mut succeeded = false;

                    for model_id in models {
                        let left = timeout.saturating_sub(started.elapsed());
                        if left.is_zero() {
                            last_error = "request timed out".into();
                            break;
                        }
                        let Some(desc) = routed.registry().get(&model_id) else {
                            continue;
                        };
                        let Ok(endpoint) = ProviderEndpoint::for_model(desc) else {
                            continue;
                        };
                        let endpoint = match endpoint.with_limits(left, 8 * 1024 * 1024) {
                            Ok(e) => e,
                            Err(e) => {
                                last_error = e.to_string();
                                continue;
                            }
                        };
                        let Ok(client) = InferenceClient::new(endpoint) else {
                            continue;
                        };

                        let mut model_req = req.clone();
                        if let Ok(p_id) = provider_model_id(desc) {
                            model_req.model = p_id.to_string();
                        }

                        let mut unveiler = StreamUnveiler::new(unveil_opt.clone());
                        let events_tx = send.clone();
                        let mut any_received = false;

                        let effective_grade = if unveil_opt.is_some() {
                            PrivacyGrade::Public
                        } else {
                            privacy
                        };

                        let result = client
                            .chat_stream(desc, effective_grade, &model_req, |ev| {
                                any_received = true;
                                for processed_ev in unveiler.feed(ev) {
                                    let _ = events_tx.send(Update::Event(processed_ev));
                                }
                            })
                            .await;

                        match result {
                            Ok(()) => {
                                succeeded = true;
                                break;
                            }
                            Err(err) => {
                                last_error = format!("{model_id}: {err}");
                                if any_received {
                                    break;
                                }
                            }
                        }
                    }

                    if !succeeded {
                        let _ = send.send(Update::Failed(last_error));
                    }
                }
            }
        });

        TurnStream {
            updates,
            stop: Box::new(move || task.abort()),
        }
    }
}

/// Asks a Nemotron model on Nebius, on a runtime the caller owns.
pub struct NebiusSource {
    client: InferenceClient,
    descriptor: ModelDescriptor,
    privacy: PrivacyGrade,
    runtime: tokio::runtime::Handle,
    wanda_access: crate::wanda::WandaVoiceAccess,
}

impl NebiusSource {
    /// `privacy` is the grade of what the person said. The client refuses to send it to a
    /// model that may not see that grade, so a cloud model with unverified retention only
    /// ever receives what was graded public.
    pub fn new(
        client: InferenceClient,
        descriptor: ModelDescriptor,
        privacy: PrivacyGrade,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            client,
            descriptor,
            privacy,
            runtime,
            wanda_access: crate::wanda::WandaVoiceAccess::default(),
        }
    }
    /// Construct the source from the selected endpoint so access proof and transport
    /// cannot accidentally refer to different providers.
    pub fn from_endpoint(
        endpoint: ProviderEndpoint,
        descriptor: ModelDescriptor,
        privacy: PrivacyGrade,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self, RuntimeError> {
        let wanda_access =
            crate::wanda::WandaVoiceAccess::from_selected(&endpoint, &descriptor, privacy);
        let mut source = Self::new(
            InferenceClient::new(endpoint)?,
            descriptor,
            privacy,
            runtime,
        );
        source.wanda_access = wanda_access;
        Ok(source)
    }
    pub fn wanda_access(&self) -> crate::wanda::WandaVoiceAccess {
        self.wanda_access
    }
}

impl TurnSource for NebiusSource {
    fn start(&mut self, request: ToolChatRequest) -> TurnStream {
        let (send, updates) = mpsc::channel();
        let client = self.client.clone();
        let descriptor = self.descriptor.clone();
        let privacy = self.privacy;
        let task = self.runtime.spawn(async move {
            let events = send.clone();
            let result = client
                .chat_stream(&descriptor, privacy, &request, move |event| {
                    let _ = events.send(Update::Event(event));
                })
                .await;
            if let Err(error) = result {
                let _ = send.send(Update::Failed(error.to_string()));
            }
        });
        TurnStream {
            updates,
            stop: Box::new(move || task.abort()),
        }
    }
}

/// How the model is asked to speak, before any particular turn.
pub const PERSONA_ES: &str = "Eres Archie, el asistente de voz de ArchieOS. Contesta en el \
idioma en que te hablaron: en español, el de Chile, natural y cercano, tuteando (nunca voseo \
como querés o tenés); en inglés si te hablan en inglés. Todo lo que escribes se dice en voz alta: frases cortas, sin \
markdown, sin listas, sin emojis, sin símbolos que no se pronuncian. Escribe los números, las \
horas y las fechas con palabras, como se dicen: son las tres y veinticinco, el veintitrés de \
septiembre, dos mil veintiséis. No inventes datos que no \
tienes, como la hora, la fecha o lo que hay en la pantalla: si no están en el contexto, dilo. \
Cuando la persona pide hacer algo en el computador, usa las herramientas en vez de explicar \
cómo se haría.";

/// The most messages of past conversation carried into a turn.
const MEMORY: usize = 12;

struct Turn {
    id: String,
    stream: TurnStream,
    /// What was released to the voice, and the tools asked for, so the exchange can be
    /// carried into the next step.
    said: Vec<String>,
    calls: Vec<(String, ToolCall)>,
    segmenter: Segmenter,
    gate: ResponseGate,
    next_sequence: u32,
    tool_calls: u32,
    text_bytes: usize,
    queue: VecDeque<CognitiveEvent>,
    ended: bool,
}

/// A [`CognitiveModelAdapter`] backed by a streamed chat model with tools.
pub struct NemotronCognitive<S: TurnSource> {
    source: S,
    provider_model: String,
    name: String,
    languages: Vec<String>,
    persona: String,
    tools: Vec<ToolDefinition>,
    context: Option<String>,
    /// The context goes with the utterance it belongs to instead of the instructions, so
    /// that what the model said earlier never reads as newer than what is true now.
    context_with_utterance: bool,
    history: Vec<ToolChatMessage>,
    turn: Option<Turn>,
    last_request: Option<ToolChatRequest>,
    /// The exchange about the utterance being answered: what the person said, then each
    /// step Archie took and what the tools answered. Reset by every new utterance.
    chain: Vec<ToolChatMessage>,
    /// Tools the last finished step asked for, waiting for their results.
    awaiting: Vec<(String, ToolCall)>,
    /// The last step to finish: its turn, and where its numbering stopped.
    ended: Option<(String, u32, u32)>,
}

impl<S: TurnSource> NemotronCognitive<S> {
    pub fn new(
        source: S,
        provider_model: impl Into<String>,
        languages: Vec<String>,
        tools: Vec<ToolDefinition>,
    ) -> Self {
        let provider_model = provider_model.into();
        Self {
            source,
            name: format!("nemotron:{provider_model}"),
            provider_model,
            languages,
            persona: PERSONA_ES.to_string(),
            tools,
            context: None,
            context_with_utterance: false,
            history: Vec::new(),
            turn: None,
            last_request: None,
            chain: Vec::new(),
            awaiting: Vec::new(),
            ended: None,
        }
    }

    /// Who the model is told it is. The live session gives it eyes and two languages.
    pub fn set_persona(&mut self, persona: impl Into<String>) {
        self.persona = persona.into();
    }

    /// Puts the context of this moment right before the utterance it belongs to — after
    /// the past conversation — rather than in the instructions ahead of it. A model reads
    /// the history as more recent than the instructions: told once "you have a phone in
    /// your hand", it said so again with the camera showing an empty room. Only the
    /// utterance being answered carries it; the history keeps what was said, bare.
    pub fn put_context_beside_utterance(&mut self) {
        self.context_with_utterance = true;
    }

    /// Tools beyond the ones it was built with (seeing and searching, in a live session).
    pub fn add_tools(&mut self, tools: impl IntoIterator<Item = ToolDefinition>) {
        self.tools.extend(tools);
    }

    /// Carries an utterance one step further, once the tools its last step asked for
    /// have been decided on.
    ///
    /// `results` answers each proposal by its correlation id: what happened, or why it did
    /// not. A proposal left unanswered is reported to the model as not done, never as done.
    /// The model then sees its own request and the outcome, and either proposes the next
    /// step — "abre firefox y busca el clima" is two — or says what it has to say.
    pub fn follow_up(
        &mut self,
        turn_id: impl Into<String>,
        results: &[(String, String)],
        budget: SpeechBudget,
    ) -> Result<(), CognitiveError> {
        let turn_id = turn_id.into();
        if let Some(running) = &self.turn {
            return Err(CognitiveError::TurnAlreadyRunning {
                running: running.id.clone(),
                requested: turn_id,
            });
        }
        if self.awaiting.is_empty() {
            return Err(CognitiveError::Backend(
                "no step is waiting for tool results".into(),
            ));
        }
        for (correlation, call) in std::mem::take(&mut self.awaiting) {
            let outcome = results
                .iter()
                .find(|(id, _)| *id == correlation)
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or_else(|| "No se hizo.".to_string());
            self.chain.push(ToolChatMessage::Tool {
                call_id: call.id,
                content: outcome,
            });
        }
        let outgoing = self.request_with(budget);
        outgoing
            .validate()
            .map_err(|reason| CognitiveError::Backend(reason.to_string()))?;
        let stream = self.source.start(outgoing.clone());
        self.last_request = Some(outgoing);
        let mut turn = Turn::new(turn_id, stream, budget);
        // The same turn carried further keeps counting where it stopped, so what it says
        // next continues the record of that turn instead of restarting it.
        if let Some((id, next_sequence, tool_calls)) = &self.ended {
            if *id == turn.id {
                turn.next_sequence = *next_sequence;
                turn.tool_calls = *tool_calls;
            }
        }
        self.turn = Some(turn);
        Ok(())
    }

    /// Whether the last finished step asked for tools that have not been answered.
    pub fn awaiting_results(&self) -> bool {
        !self.awaiting.is_empty()
    }

    /// Facts for the next turns that the model cannot know: the time, the date, which
    /// application is in front. Cerebra knows them; the model only guesses.
    pub fn set_context(&mut self, context: impl Into<String>) {
        let context = context.into();
        self.context = (!context.trim().is_empty()).then_some(context);
    }

    /// Records a finished exchange as it was *heard*, so the next turn follows from it.
    ///
    /// The caller passes what the voice actually said, not what was generated: after an
    /// interruption those differ, and a model that remembers the unsaid half will later
    /// refer to it as if the person had heard it.
    pub fn remember(&mut self, said_by_person: &str, heard_from_archie: &str) {
        if said_by_person.trim().is_empty() {
            return;
        }
        self.history
            .push(ToolChatMessage::User(said_by_person.to_string()));
        if !heard_from_archie.trim().is_empty() {
            self.history.push(ToolChatMessage::Assistant {
                content: Some(heard_from_archie.to_string()),
                tool_calls: Vec::new(),
            });
        }
        if self.history.len() > MEMORY {
            let excess = self.history.len() - MEMORY;
            self.history.drain(..excess);
        }
    }

    /// The last request sent, for tests and for anyone auditing what left the machine.
    pub fn last_request(&self) -> Option<&ToolChatRequest> {
        self.last_request.as_ref()
    }

    fn request_with(&self, budget: SpeechBudget) -> ToolChatRequest {
        let length = match budget {
            SpeechBudget::Sentence => "Responde en una sola frase.",
            // Spoken, a full paragraph is twenty seconds of not being able to get a word
            // in. The answer comes first and short. How much more to say is read from the
            // conversation, not only from being asked: a passing doubt gets a sentence, a
            // subject the person is plainly working through gets a few. The conversation
            // kept in `history` is what a follow-up continues.
            _ => {
                "Hablas, no escribes: lo que digas se oye, y la persona no puede interrumpir \
                 un párrafo. Responde primero lo esencial en una o dos frases cortas, de no \
                 más de cuarenta palabras. Nunca uses listas, numeración ni varios puntos. \
                 Decide por el contexto si seguir: ante una duda puntual, detente ahí; si la \
                 persona pide explicación, viene profundizando en el tema o lo que dijiste \
                 quedaría incompleto sin más, agrega hasta tres frases y detente. Siempre \
                 puede pedirte que sigas."
            }
        };
        let mut system = format!("{}\n\n{length}", self.persona);
        let beside = self.context_with_utterance;
        if let (Some(context), false) = (&self.context, beside) {
            system.push_str("\n\nContexto de este momento, que puedes usar como cierto:\n");
            system.push_str(context);
        }
        let mut messages = vec![ToolChatMessage::System(system)];
        messages.extend(self.history.iter().cloned());
        let mut chain = self.chain.iter().cloned();
        if let (Some(context), true) = (&self.context, beside) {
            match chain.next() {
                Some(ToolChatMessage::User(said)) => messages.push(ToolChatMessage::User(format!(
                    "[Contexto de este momento, que no dijo la persona y vale más que lo \
                     dicho antes en la conversación:\n{context}]\n\n{said}"
                ))),
                Some(other) => messages.push(other),
                None => {}
            }
        }
        messages.extend(chain);
        ToolChatRequest {
            model: self.provider_model.clone(),
            messages,
            tools: self.tools.clone(),
            max_tokens: if matches!(budget, SpeechBudget::Sentence) {
                96
            } else {
                400
            },
            temperature: 0.3,
        }
    }
}

impl<S: TurnSource> CognitiveModelAdapter for NemotronCognitive<S> {
    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn begin(&mut self, request: CognitiveRequest) -> Result<(), CognitiveError> {
        if let Some(running) = &self.turn {
            return Err(CognitiveError::TurnAlreadyRunning {
                running: running.id.clone(),
                requested: request.turn_id,
            });
        }
        validate_request(&request, &self.languages)?;
        // A new utterance starts a new exchange; anything the last one left waiting is moot.
        self.chain = vec![ToolChatMessage::User(request.transcript.clone())];
        self.awaiting.clear();
        let outgoing = self.request_with(request.budget);
        outgoing
            .validate()
            .map_err(|reason| CognitiveError::Backend(reason.to_string()))?;
        let stream = self.source.start(outgoing.clone());
        self.last_request = Some(outgoing);
        self.turn = Some(Turn::new(request.turn_id, stream, request.budget));
        Ok(())
    }

    fn poll(&mut self) -> Option<CognitiveEvent> {
        let name = self.name.clone();
        let turn = self.turn.as_mut()?;
        if turn.queue.is_empty() && !turn.ended {
            for spoken in turn.segmenter.push_up_to("", 1) {
                release(turn, spoken);
            }
        }
        // Leave transport output pending once an event is ready. Also cap invisible
        // fragments per tick so an unfinished/empty burst cannot monopolise hearing.
        for _ in 0..32 {
            if !turn.queue.is_empty() || turn.ended {
                break;
            }
            match turn.stream.updates.try_recv() {
                Ok(Update::Event(StreamEvent::Text(words))) => {
                    // Normal speech is requested with a 400-token budget. Refuse a
                    // misbehaving backend before one huge delta/buffer can stall ASR.
                    turn.text_bytes = turn.text_bytes.saturating_add(words.len());
                    if words.len() > 16 * 1024 || turn.text_bytes > 64 * 1024 {
                        (turn.stream.stop)();
                        turn.queue.push_back(CognitiveEvent::Failed {
                            turn_id: turn.id.clone(),
                            reason: "the answer exceeded the realtime text limit".into(),
                        });
                        turn.ended = true;
                        break;
                    }
                    for spoken in turn.segmenter.push_up_to(&words, 1) {
                        release(turn, spoken);
                    }
                }
                Ok(Update::Event(StreamEvent::ToolCall(call))) => {
                    let correlation_id = format!("{}#{}", turn.id, turn.tool_calls);
                    turn.tool_calls += 1;
                    turn.calls.push((correlation_id.clone(), call.clone()));
                    turn.queue
                        .push_back(CognitiveEvent::ToolIntent(SpeechToolIntentCandidate {
                            correlation_id,
                            tool_name: call.name,
                            raw_arguments: call.arguments,
                            source_adapter: name.clone(),
                            confidence: None,
                        }));
                }
                Ok(Update::Event(StreamEvent::Finished { .. })) => {
                    if !turn.ended {
                        if let Some(spoken) = turn.segmenter.finish() {
                            release(turn, spoken);
                        }
                        turn.queue.push_back(CognitiveEvent::Completed {
                            turn_id: turn.id.clone(),
                        });
                        turn.ended = true;
                    }
                }
                Ok(Update::Failed(reason)) => {
                    if !turn.ended {
                        turn.queue.push_back(CognitiveEvent::Failed {
                            turn_id: turn.id.clone(),
                            reason,
                        });
                        turn.ended = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !turn.ended {
                        turn.queue.push_back(CognitiveEvent::Failed {
                            turn_id: turn.id.clone(),
                            reason: "the answer stopped arriving before it finished".into(),
                        });
                        turn.ended = true;
                    }
                    break;
                }
            }
        }
        let event = turn.queue.pop_front();
        if turn.ended && turn.queue.is_empty() {
            if let Some(done) = self.turn.take() {
                if !done.said.is_empty() || !done.calls.is_empty() {
                    self.chain.push(ToolChatMessage::Assistant {
                        content: (!done.said.is_empty()).then(|| done.said.join(" ")),
                        tool_calls: done.calls.iter().map(|(_, call)| call.clone()).collect(),
                    });
                }
                self.ended = Some((done.id.clone(), done.next_sequence, done.tool_calls));
                self.awaiting = done.calls;
            }
        }
        event
    }

    fn cancel(&mut self, turn_id: &str, _reason: CancelReason) {
        if self.turn.as_ref().is_some_and(|turn| turn.id == turn_id) {
            if let Some(mut turn) = self.turn.take() {
                (turn.stream.stop)();
            }
        }
    }

    fn running_turn(&self) -> Option<&str> {
        self.turn.as_ref().map(|turn| turn.id.as_str())
    }
}

/// Offers a piece of the answer to the budget, and queues it if the budget lets it out.
fn release(turn: &mut Turn, spoken: Spoken) {
    let segment = ResponseSegment {
        turn_id: turn.id.clone(),
        sequence: turn.next_sequence,
        text: spoken.text,
        source: SpokenContentSource::CognitiveModel,
        ends_sentence: spoken.ends_sentence,
    };
    if turn.gate.admit(&segment) {
        turn.next_sequence += 1;
        turn.said.push(segment.text.clone());
        turn.queue.push_back(CognitiveEvent::Segment(segment));
    }
}

impl Turn {
    fn new(id: String, stream: TurnStream, budget: SpeechBudget) -> Self {
        Self {
            id,
            stream,
            said: Vec::new(),
            calls: Vec::new(),
            segmenter: Segmenter::default(),
            gate: ResponseGate::new(budget),
            next_sequence: 0,
            tool_calls: 0,
            text_bytes: 0,
            queue: VecDeque::new(),
            ended: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_model_runtime::SseAssembler;
    use std::sync::{Arc, Mutex};

    struct Burst(bool);
    impl TurnSource for Burst {
        fn start(&mut self, _: ToolChatRequest) -> TurnStream {
            let (send, updates) = mpsc::channel();
            for _ in 0..if self.0 { 1 } else { 1000 } {
                send.send(Update::Event(StreamEvent::Text(
                    "Una frase. ".repeat(if self.0 { 1000 } else { 1 }),
                )))
                .unwrap();
            }
            send.send(Update::Event(StreamEvent::Finished {
                reason: Some("stop".into()),
            }))
            .unwrap();
            TurnStream {
                updates,
                stop: Box::new(|| {}),
            }
        }
    }

    #[test]
    fn one_poll_does_not_drain_a_prequeued_transport_burst() {
        let mut cognitive = NemotronCognitive::new(Burst(false), "test", vec!["es".into()], vec![]);
        cognitive
            .begin(ask("burst", "hola", SpeechBudget::Full))
            .unwrap();
        assert!(matches!(cognitive.poll(), Some(CognitiveEvent::Segment(_))));
        let turn = cognitive.turn.as_ref().expect("transport remains pending");
        assert_eq!(
            turn.next_sequence, 1,
            "one released segment per poll, without parsing the whole backlog"
        );
        assert!(turn.queue.is_empty());
        let rest = drain(&mut cognitive);
        assert_eq!(
            rest.iter()
                .filter(|event| matches!(event, CognitiveEvent::Segment(_)))
                .count(),
            999
        );
        assert!(matches!(
            rest.last(),
            Some(CognitiveEvent::Completed { .. })
        ));
    }

    #[test]
    fn one_poll_does_not_segment_a_whole_answer_delivered_in_one_update() {
        let mut cognitive = NemotronCognitive::new(Burst(true), "test", vec!["es".into()], vec![]);
        cognitive
            .begin(ask("combined", "hola", SpeechBudget::Full))
            .unwrap();
        assert!(matches!(cognitive.poll(), Some(CognitiveEvent::Segment(_))));
        let turn = cognitive.turn.as_ref().unwrap();
        assert_eq!(turn.next_sequence, 1);
        assert!(turn.queue.is_empty());
        assert_eq!(
            drain(&mut cognitive)
                .iter()
                .filter(|event| matches!(event, CognitiveEvent::Segment(_)))
                .count(),
            999
        );
    }

    const TEXT: &[u8] = include_bytes!("../../archie-model-runtime/tests/fixtures/nebius-text.sse");
    const TOOL: &[u8] =
        include_bytes!("../../archie-model-runtime/tests/fixtures/nebius-tool-call.sse");
    const REASONING: &[u8] =
        include_bytes!("../../archie-model-runtime/tests/fixtures/nebius-reasoning.sse");

    /// Replays a recorded provider stream, and remembers whether it was told to stop.
    struct Replay {
        bytes: &'static [u8],
        stopped: Arc<Mutex<bool>>,
        hold: bool,
    }

    impl Replay {
        fn of(bytes: &'static [u8]) -> Self {
            Self {
                bytes,
                stopped: Arc::new(Mutex::new(false)),
                hold: false,
            }
        }
    }

    impl TurnSource for Replay {
        fn start(&mut self, _request: ToolChatRequest) -> TurnStream {
            let (send, updates) = mpsc::channel();
            let mut assembler = SseAssembler::new();
            for event in assembler.feed(self.bytes).unwrap() {
                send.send(Update::Event(event)).unwrap();
            }
            for event in assembler.close().unwrap() {
                send.send(Update::Event(event)).unwrap();
            }
            let stopped = self.stopped.clone();
            // A held stream keeps its sender alive, as a real one does while it runs.
            let keep = if self.hold { Some(send) } else { None };
            TurnStream {
                updates,
                stop: Box::new(move || {
                    let _ = &keep;
                    *stopped.lock().unwrap() = true;
                }),
            }
        }
    }

    fn adapter(bytes: &'static [u8]) -> NemotronCognitive<Replay> {
        NemotronCognitive::new(
            Replay::of(bytes),
            "nvidia/Nemotron-3_5-Lightning",
            vec!["es".into(), "en".into()],
            crate::tools::os_control(),
        )
    }

    fn ask(turn: &str, words: &str, budget: SpeechBudget) -> CognitiveRequest {
        CognitiveRequest {
            turn_id: turn.into(),
            transcript: words.into(),
            language: "es-CL".into(),
            budget,
        }
    }

    fn drain<S: TurnSource>(adapter: &mut NemotronCognitive<S>) -> Vec<CognitiveEvent> {
        let mut out = Vec::new();
        while let Some(event) = adapter.poll() {
            out.push(event);
        }
        out
    }

    #[test]
    fn an_answer_arrives_as_numbered_sentences_and_then_completes() {
        let mut cognitive = adapter(TEXT);
        cognitive
            .begin(ask("t1", "qué es un volcán", SpeechBudget::Full))
            .unwrap();
        let events = drain(&mut cognitive);
        let segments: Vec<&ResponseSegment> = events
            .iter()
            .filter_map(|event| match event {
                CognitiveEvent::Segment(segment) => Some(segment),
                _ => None,
            })
            .collect();
        assert!(
            segments.len() >= 2,
            "a real answer is more than one piece: {events:?}"
        );
        for (at, segment) in segments.iter().enumerate() {
            assert_eq!(
                segment.sequence, at as u32,
                "numbered from zero without gaps"
            );
            assert_eq!(segment.source, SpokenContentSource::CognitiveModel);
            assert!(
                segment.text.chars().any(char::is_alphanumeric),
                "every piece is sayable"
            );
        }
        assert_eq!(
            events.last(),
            Some(&CognitiveEvent::Completed {
                turn_id: "t1".into()
            })
        );
        assert_eq!(cognitive.running_turn(), None, "and the turn is over");
    }

    #[test]
    fn granted_one_sentence_the_voice_gets_exactly_one_complete_sentence() {
        let mut cognitive = adapter(TEXT);
        cognitive
            .begin(ask("t1", "qué es un volcán", SpeechBudget::Sentence))
            .unwrap();
        let segments: Vec<ResponseSegment> = drain(&mut cognitive)
            .into_iter()
            .filter_map(|event| match event {
                CognitiveEvent::Segment(segment) => Some(segment),
                _ => None,
            })
            .collect();
        let sentences = segments
            .iter()
            .filter(|segment| segment.ends_sentence)
            .count();
        assert_eq!(sentences, 1, "one sentence, complete: {segments:?}");
        assert!(
            segments.last().unwrap().ends_sentence,
            "and it is the last thing said"
        );
    }

    #[test]
    fn a_tool_the_model_asks_for_is_passed_on_as_a_proposal() {
        let mut cognitive = adapter(TOOL);
        cognitive
            .begin(ask("t7", "abre firefox po", SpeechBudget::Full))
            .unwrap();
        let events = drain(&mut cognitive);
        let intents: Vec<&SpeechToolIntentCandidate> = events
            .iter()
            .filter_map(|event| match event {
                CognitiveEvent::ToolIntent(intent) => Some(intent),
                _ => None,
            })
            .collect();
        assert_eq!(intents.len(), 1, "{events:?}");
        assert_eq!(intents[0].tool_name, "open_app");
        assert!(intents[0].raw_arguments.to_lowercase().contains("firefox"));
        assert_eq!(
            intents[0].correlation_id, "t7#0",
            "correlated to the utterance"
        );
        assert!(intents[0].source_adapter.starts_with("nemotron:"));
        assert!(matches!(
            events.last(),
            Some(CognitiveEvent::Completed { .. })
        ));
    }

    #[test]
    fn a_stopped_turn_says_nothing_more_not_even_that_it_completed() {
        let mut cognitive = adapter(TEXT);
        cognitive.source.hold = true;
        let stopped = cognitive.source.stopped.clone();
        cognitive
            .begin(ask("t2", "qué es un volcán", SpeechBudget::Full))
            .unwrap();
        let first = cognitive.poll();
        assert!(matches!(first, Some(CognitiveEvent::Segment(_))));
        cognitive.cancel("t2", CancelReason::Interrupted);
        assert!(*stopped.lock().unwrap(), "the stream itself was stopped");
        assert_eq!(
            cognitive.poll(),
            None,
            "nothing further for the stopped turn"
        );
        assert_eq!(cognitive.running_turn(), None);
    }

    #[test]
    fn a_second_turn_does_not_silently_replace_the_first() {
        let mut cognitive = adapter(TEXT);
        cognitive.source.hold = true;
        cognitive
            .begin(ask("t3", "qué es un volcán", SpeechBudget::Full))
            .unwrap();
        let refused = cognitive.begin(ask("t4", "y un glaciar", SpeechBudget::Full));
        assert!(matches!(
            refused,
            Err(CognitiveError::TurnAlreadyRunning { .. })
        ));
    }

    #[test]
    fn it_refuses_what_it_cannot_answer_before_sending_anything() {
        let mut cognitive = adapter(TEXT);
        assert_eq!(
            cognitive.begin(ask("t5", "   ", SpeechBudget::Full)),
            Err(CognitiveError::EmptyTranscript)
        );
        assert!(cognitive
            .begin(ask("t5", "hola", SpeechBudget::Token))
            .is_err());
        let mut german = ask("t5", "hallo", SpeechBudget::Full);
        german.language = "de-DE".into();
        assert!(matches!(
            cognitive.begin(german),
            Err(CognitiveError::UnsupportedLanguage { .. })
        ));
        assert!(
            cognitive.last_request().is_none(),
            "none of that left the machine"
        );
    }

    #[test]
    fn the_context_of_this_moment_comes_after_what_was_said_before() {
        let mut cognitive = adapter(TEXT);
        cognitive.put_context_beside_utterance();
        cognitive.remember("¿qué tengo en la mano?", "Tienes un celular en la mano.");
        cognitive.set_context("Ahora mismo no se ve a ninguna persona.");
        cognitive
            .begin(ask("t9", "¿y ahora?", SpeechBudget::Full))
            .unwrap();
        let sent = cognitive.last_request().unwrap();
        assert!(
            matches!(&sent.messages[0], ToolChatMessage::System(text) if !text.contains("ninguna persona")),
            "the moment is not in the instructions, ahead of the history"
        );
        assert!(matches!(&sent.messages[3], ToolChatMessage::User(text)
            if text.contains("ninguna persona") && text.ends_with("¿y ahora?")));
        cognitive.cancel("t9", CancelReason::Interrupted);
        cognitive.remember("¿y ahora?", "No veo a nadie.");
        assert!(
            matches!(&cognitive.history[2], ToolChatMessage::User(text) if text == "¿y ahora?"),
            "the history keeps what was said, without the context of that moment"
        );
    }

    #[test]
    fn the_model_is_told_the_facts_it_would_otherwise_guess_and_what_was_heard_before() {
        let mut cognitive = adapter(TEXT);
        cognitive.set_context("Hora local: 14:02, martes 23 de septiembre de 2026.");
        cognitive.remember("hola archie", "Hola, ¿en qué te ayudo?");
        cognitive
            .begin(ask("t6", "qué hora es", SpeechBudget::Sentence))
            .unwrap();
        let sent = cognitive.last_request().unwrap();
        let system = match &sent.messages[0] {
            ToolChatMessage::System(text) => text.clone(),
            other => panic!("the first message is the system prompt, not {other:?}"),
        };
        assert!(system.contains("14:02"), "the clock is given, not guessed");
        assert!(
            system.contains("No inventes"),
            "and it is told not to invent"
        );
        assert!(system.contains("una sola frase"), "and how much it may say");
        assert_eq!(
            sent.messages.len(),
            4,
            "system, the past exchange, and this question"
        );
        assert!(matches!(&sent.messages[3], ToolChatMessage::User(text) if text == "qué hora es"));
        assert_eq!(sent.tools.len(), crate::tools::os_control().len());
        assert_eq!(
            sent.max_tokens, 96,
            "one sentence does not need four hundred tokens"
        );
    }

    #[test]
    fn a_turn_carried_further_keeps_counting_where_it_stopped() {
        let mut cognitive = adapter(TOOL);
        cognitive
            .begin(ask("t11", "abre firefox", SpeechBudget::Full))
            .unwrap();
        drain(&mut cognitive);
        cognitive
            .follow_up(
                "t11",
                &[("t11#0".into(), "No se hizo.".into())],
                SpeechBudget::Full,
            )
            .unwrap();
        let second: Vec<String> = drain(&mut cognitive)
            .into_iter()
            .filter_map(|event| match event {
                CognitiveEvent::ToolIntent(intent) => Some(intent.correlation_id),
                _ => None,
            })
            .collect();
        assert_eq!(second, vec!["t11#1".to_string()], "not a second t11#0");

        cognitive.source.bytes = TEXT;
        cognitive.follow_up("t11", &[], SpeechBudget::Full).unwrap();
        let first_said = drain(&mut cognitive)
            .into_iter()
            .find_map(|event| match event {
                CognitiveEvent::Segment(segment) => Some(segment.sequence),
                _ => None,
            });
        assert_eq!(
            first_said,
            Some(0),
            "nothing had been said, so speech starts at 0"
        );
    }

    #[test]
    fn the_next_step_sees_what_was_asked_for_and_what_came_of_it() {
        let mut cognitive = adapter(TOOL);
        assert!(
            cognitive.follow_up("t9", &[], SpeechBudget::Full).is_err(),
            "nothing to follow up before anything was asked"
        );
        cognitive
            .begin(ask("t9", "abre firefox po", SpeechBudget::Full))
            .unwrap();
        drain(&mut cognitive);
        assert!(cognitive.awaiting_results());
        cognitive
            .follow_up(
                "t9.1",
                &[("t9#0".into(), "Firefox está abierto y al frente.".into())],
                SpeechBudget::Full,
            )
            .unwrap();
        let sent = cognitive.last_request().unwrap();
        assert_eq!(
            sent.messages.len(),
            4,
            "system, the question, the step, its result"
        );
        assert!(
            matches!(&sent.messages[1], ToolChatMessage::User(text) if text == "abre firefox po")
        );
        let asked = match &sent.messages[2] {
            ToolChatMessage::Assistant { tool_calls, .. } => tool_calls.clone(),
            other => panic!("the step Archie took, not {other:?}"),
        };
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].name, "open_app");
        assert!(matches!(
            &sent.messages[3],
            ToolChatMessage::Tool { call_id, content }
                if *call_id == asked[0].id && content.contains("abierto")
        ));
        assert!(!cognitive.awaiting_results(), "each result is given once");

        // An unanswered proposal is reported as not done, never as done.
        drain(&mut cognitive);
        cognitive
            .follow_up("t9.2", &[], SpeechBudget::Full)
            .unwrap();
        let sent = cognitive.last_request().unwrap();
        assert!(matches!(
            sent.messages.last(),
            Some(ToolChatMessage::Tool { content, .. }) if content == "No se hizo."
        ));

        // And a new utterance starts over.
        drain(&mut cognitive);
        cognitive
            .begin(ask("t10", "gracias", SpeechBudget::Full))
            .unwrap();
        assert_eq!(cognitive.last_request().unwrap().messages.len(), 2);
    }

    #[test]
    fn a_reasoning_models_notes_never_reach_the_voice() {
        let mut cognitive = adapter(REASONING);
        cognitive
            .begin(ask("t8", "qué es un volcán", SpeechBudget::Full))
            .unwrap();
        let said: String = drain(&mut cognitive)
            .into_iter()
            .filter_map(|event| match event {
                CognitiveEvent::Segment(segment) => Some(segment.text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        let lowered = said.to_lowercase();
        for note in [
            "the user",
            "we need",
            "let's",
            "respond in spanish",
            "okay,",
        ] {
            assert!(
                !lowered.contains(note),
                "a reasoning note reached the voice ({note:?}): {said}"
            );
        }
    }
}
