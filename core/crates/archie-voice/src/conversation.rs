//! A spoken conversation: the ears, the floor, the model, the hands and the voice, in one
//! loop.
//!
//! ```text
//!  microphone ─► ears (Nemotron 3.5 ASR) ─► floor ─► Nemotron on Nebius ─► voice (Magpie) ─► speaker
//!                                               │            │
//!                                               │            └─► proposals ─► decide ─► hands ─┐
//!                                               └───────────────── results told back ◄─────────┘
//! ```
//!
//! The loop owns the listening and the deciding; the voice runs on its own thread, because
//! making speech takes longer than hearing it and the ears must not go deaf meanwhile.
//! Three things the loop is careful about:
//!
//! - **It answers what was said to it.** Whether to speak is the speech plane's
//!   [`FloorPolicy`](archie_speech::FloorPolicy), fed by [`crate::listen`]: an utterance
//!   that neither names Archie nor answers it is heard and left alone.
//! - **It does not hear itself.** Without echo cancellation a microphone near a speaker
//!   hears the voice, and a voice that answers itself never stops. Unless the person says
//!   they wear headphones, the ears are closed while Archie speaks.
//! - **It stops when spoken over** — with headphones, where it can tell. The queued audio
//!   is dropped, the turn is cancelled as an interruption, and what was not said stays in
//!   the commit log, unsaid.

use crate::audio::{loudness, AudioError, Microphone, Speaker, VoiceActivity};
use crate::listen::{after_utterance, over_archie, Utterance};
use crate::native::Ears;
use crate::nemotron::{NemotronCognitive, TurnSource};
use crate::voice::Speaks;
use crate::wav::{read_wav, write_wav};
use archie_orb::{publish, semantic_features, signal, SignalKind};
use archie_speech::{
    FloorAction, SessionEvent, SpeechBudget, SpeechSession, SpeechToolIntentCandidate,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where the sound comes from.
pub enum Hearing {
    Microphone(Microphone),
    /// A recording, played in as if it were the room, then silence long enough for the
    /// conversation that follows it to finish.
    Recording {
        samples: Vec<f32>,
        rate: u32,
        at: usize,
        silence_left: usize,
    },
}

impl Hearing {
    /// A mono 16-bit PCM WAV file, after a second of quiet — a room is quiet before
    /// somebody speaks, and a streaming recogniser given words at its very first frame
    /// loses them.
    pub fn recording(path: &std::path::Path, then_silence: Duration) -> Result<Self, String> {
        let (spoken, rate) = read_wav(path)?;
        let mut samples = vec![0.0; rate as usize];
        samples.extend(spoken);
        Ok(Self::Recording {
            samples,
            rate,
            at: 0,
            silence_left: (then_silence.as_secs_f32() * rate as f32) as usize,
        })
    }

    /// The next 20 ms of sound and its rate, or `None` when a recording has run out.
    fn frame(&mut self) -> Result<Option<(Vec<f32>, u32)>, AudioError> {
        match self {
            Self::Microphone(microphone) => Ok(Some((
                microphone.frame()?.to_vec(),
                crate::audio::CAPTURE_RATE,
            ))),
            Self::Recording {
                samples,
                rate,
                at,
                silence_left,
            } => {
                let step = (*rate / 50) as usize;
                if *at < samples.len() {
                    let end = (*at + step).min(samples.len());
                    let frame = samples[*at..end].to_vec();
                    *at = end;
                    return Ok(Some((frame, *rate)));
                }
                if *silence_left == 0 {
                    return Ok(None);
                }
                let n = step.min(*silence_left);
                *silence_left -= n;
                Ok(Some((vec![0.0; n], *rate)))
            }
        }
    }

    fn is_live(&self) -> bool {
        matches!(self, Self::Microphone(_))
    }
}

/// Where the voice goes.
pub enum Saying {
    Speaker(Speaker),
    Recording {
        path: PathBuf,
        samples: Vec<i16>,
        rate: u32,
    },
}

impl Saying {
    fn play(&mut self, pcm: &[i16]) -> Result<(), AudioError> {
        match self {
            Self::Speaker(speaker) => speaker.play(pcm),
            Self::Recording { samples, .. } => {
                samples.extend_from_slice(pcm);
                Ok(())
            }
        }
    }

    fn settle(&mut self) {
        match self {
            Self::Speaker(speaker) => {
                let _ = speaker.drain();
            }
            Self::Recording {
                path,
                samples,
                rate,
            } => {
                let _ = write_wav(path, samples, *rate);
            }
        }
    }

    fn hush(&mut self) {
        if let Self::Speaker(speaker) = self {
            let _ = speaker.stop();
        }
    }
}

/// How a conversation is set up.
pub struct Setup {
    /// The locale the ears transcribe (`es-US`, `es-ES`, or `auto`).
    pub listen_language: String,
    /// Answer without being named. For a person alone with their computer.
    pub always: bool,
    /// The person wears headphones: the ears stay open while Archie speaks, so being
    /// spoken over can stop it.
    pub headphones: bool,
    /// How long a silence after words ends the utterance, when the ears have not.
    pub end_after: Duration,
    /// The most steps one utterance may take through tools.
    pub steps: usize,
    /// Words the ears should favour, and how strongly: the name first of all.
    pub phrases: Vec<String>,
    pub phrase_boost: f32,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            listen_language: "es-US".into(),
            always: false,
            headphones: false,
            end_after: Duration::from_millis(800),
            steps: 6,
            phrases: vec!["Archie".into()],
            phrase_boost: 5.0,
        }
    }
}

/// What the loop reports as it goes: printed by the command line, kept by a test.
#[derive(Debug, Clone, PartialEq)]
pub enum Moment {
    Heard {
        text: String,
        at_ms: u128,
    },
    Floor {
        action: String,
        reason: String,
        evidence: Vec<String>,
    },
    Said {
        text: String,
        first_audio_ms: f64,
        audio_s: f64,
        stopped: bool,
    },
    Proposed {
        tool: String,
        arguments: String,
    },
    Did {
        report: String,
    },
    Interrupted,
    Alert(String),
    /// About ten times a second: how loud the room is, and whether that is a voice.
    Level {
        level: f32,
        voiced: bool,
    },
    /// The voice began saying this.
    Speaking {
        text: String,
    },
    /// Nothing is being answered or said: back to listening.
    Ready,
}

enum ToVoice {
    /// Said only while `generation` is still current.
    Say {
        generation: u64,
        sequence: u32,
        text: String,
        semantic_features: Vec<f32>,
    },
    /// Everything queued so far is dropped.
    Hush,
    Finish,
}

enum FromVoice {
    Began {
        text: String,
    },
    Spoke {
        sequence: u32,
        text: String,
        first_audio_ms: f64,
        audio_s: f64,
        stopped: bool,
    },
    Failed(String),
    Idle,
}

/// Runs the conversation until a recording runs out, or `stop` is set.
///
/// `act` carries out one proposal and says what came of it; `context` says what the model
/// should know right now (the time, the application in front).
#[allow(clippy::too_many_arguments)]
pub fn converse<S: TurnSource>(
    setup: &Setup,
    ears: &Ears,
    voice: Box<dyn Speaks>,
    mut hearing: Hearing,
    saying: Saying,
    session: &mut SpeechSession<NemotronCognitive<S>>,
    act: &mut dyn FnMut(&SpeechToolIntentCandidate) -> String,
    context: &mut dyn FnMut() -> String,
    stop: Arc<AtomicBool>,
    report: &mut dyn FnMut(Moment),
) -> Result<(), String> {
    let started = Instant::now();
    // Bumped on every interruption: audio of an older generation is never played.
    let generation = Arc::new(AtomicU64::new(0));
    let (to_voice, voice_inbox) = mpsc::channel::<ToVoice>();
    let (voice_outbox, from_voice) = mpsc::channel::<FromVoice>();
    let voice_thread = {
        let generation = generation.clone();
        std::thread::Builder::new()
            .name("archie-voice".into())
            .spawn(move || speak(voice, saying, voice_inbox, voice_outbox, generation))
            .map_err(|error| error.to_string())?
    };

    let phrases: Vec<&str> = setup.phrases.iter().map(String::as_str).collect();
    let mut listening = ears
        .listen(&setup.listen_language, &phrases, setup.phrase_boost)
        .map_err(|e| e.to_string())?;
    publish(&signal(SignalKind::Listen {
        active: true,
        audio_envelope: 0.0,
    }));
    let mut activity = VoiceActivity::default();
    let mut partial = String::new();
    let mut audio_ms: u64 = 0;
    let mut last_words_ms: u64 = 0;
    let mut speaking = 0usize; // segments given to the voice and not yet reported
    let mut last_spoke: Option<Instant> = None;
    let mut turn_number = 0u32;
    let mut turn: Option<Answering> = None;
    let mut forced = false;
    let mut frames = 0u64;
    let mut was_busy = false;
    let trace = std::env::var_os("ARCHIE_VOICE_TRACE").is_some();
    // Audio goes to the ears a model step at a time: asking after every 20 ms frame made
    // the same recogniser three times slower than the runtime's own benchmark.
    let mut pending: Vec<f32> = Vec::new();

    let result = (|| -> Result<(), String> {
        while !stop.load(Ordering::Relaxed) {
            let Some((frame, rate)) = hearing.frame().map_err(|e| e.to_string())? else {
                // A recording ran out: let what is under way finish, then stop.
                if turn.is_none() && speaking == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
                pump_voice(&from_voice, session, &mut speaking, &mut last_spoke, report)?;
                advance(
                    session,
                    &mut turn,
                    setup,
                    act,
                    context,
                    &to_voice,
                    &generation,
                    &mut speaking,
                    report,
                )?;
                continue;
            };
            audio_ms += (frame.len() as u64 * 1000) / rate.max(1) as u64;
            let level = loudness(&frame);
            let voiced = activity.hear(level);
            frames += 1;
            if frames % 5 == 0 {
                report(Moment::Level { level, voiced });
            }

            let archie_talking = speaking > 0;
            if archie_talking && setup.headphones && voiced {
                let decision = session.floor_moved(&over_archie(&partial));
                if decision.action == FloorAction::Yield {
                    generation.fetch_add(1, Ordering::SeqCst);
                    let _ = to_voice.send(ToVoice::Hush);
                    report(Moment::Interrupted);
                    publish(&signal(SignalKind::Think {
                        active: false,
                        trace_id: format!("voz-{turn_number}"),
                        working_set: BTreeMap::new(),
                    }));
                    turn = None;
                }
            }
            // Closed ears while the voice plays through a speaker: otherwise it hears itself.
            if !archie_talking || setup.headphones {
                pending.extend_from_slice(&frame);
            }
            let step = (rate as usize * 160) / 1000;
            let ready = pending.len() >= step;
            if ready {
                listening.push(&pending, rate).map_err(|e| e.to_string())?;
                pending.clear();
            }

            // What the ears have made of it.
            while let Some(heard) = if ready || forced {
                listening.next().map_err(|e| e.to_string())?
            } else {
                None
            } {
                if trace && (heard.is_final || heard.text.trim() != partial) {
                    eprintln!(
                        "[ears {audio_ms} ms] {} {:?}",
                        if heard.is_final { "final  " } else { "interim" },
                        heard.text
                    );
                }
                let text = heard.text.trim().to_string();
                if !text.is_empty() && text != partial {
                    partial = text.clone();
                    last_words_ms = audio_ms;
                }
                if heard.is_final && !partial.is_empty() {
                    let silence_ms = audio_ms.saturating_sub(last_words_ms) as u32;
                    let said = tidy(&std::mem::take(&mut partial));
                    report(Moment::Heard {
                        text: said.clone(),
                        at_ms: started.elapsed().as_millis(),
                    });
                    forced = false;
                    if turn.is_some() || speaking > 0 {
                        // Busy answering the last thing: this one is heard, not answered.
                        continue;
                    }
                    let mut state = after_utterance(
                        &Utterance {
                            text: said.clone(),
                            final_by_recogniser: true,
                            silence_ms: silence_ms.max(setup.end_after.as_millis() as u32),
                        },
                        last_spoke.map(|at| at.elapsed()),
                    );
                    if setup.always {
                        state.response_utility = archie_speech::Probability::new(0.9);
                        state.evidence.push("answering everything: --always".into());
                    }
                    session.cognitive_mut().set_context(context());
                    turn_number += 1;
                    let id = format!("voz-{turn_number}");
                    let decision = session
                        .heard(&id, &said, &state)
                        .map_err(|e| e.to_string())?;
                    report(Moment::Floor {
                        action: format!("{:?}", decision.action),
                        reason: decision.reason.clone(),
                        evidence: state.evidence.clone(),
                    });
                    if matches!(decision.budget, SpeechBudget::Sentence | SpeechBudget::Full) {
                        publish(&signal(SignalKind::Think {
                            active: true,
                            trace_id: id.clone(),
                            working_set: BTreeMap::new(),
                        }));
                        turn = Some(Answering {
                            id,
                            heard: said,
                            budget: decision.budget,
                            steps: 1,
                            results: Vec::new(),
                        });
                    }
                }
            }
            // The ears did not call it, but the person has plainly stopped.
            if !forced
                && !partial.is_empty()
                && activity.quiet_ms() as u64 >= setup.end_after.as_millis() as u64
                && audio_ms.saturating_sub(last_words_ms) >= setup.end_after.as_millis() as u64
            {
                listening.end_utterance().map_err(|e| e.to_string())?;
                forced = true;
            }

            pump_voice(&from_voice, session, &mut speaking, &mut last_spoke, report)?;
            advance(
                session,
                &mut turn,
                setup,
                act,
                context,
                &to_voice,
                &generation,
                &mut speaking,
                report,
            )?;
            let busy = turn.is_some() || speaking > 0;
            if was_busy && !busy {
                report(Moment::Ready);
            }
            was_busy = busy;

            if !hearing.is_live() {
                // A recording is read faster than real time; the voice thread still gets
                // the processor it needs.
                std::thread::yield_now();
            }
        }
        Ok(())
    })();

    let _ = to_voice.send(ToVoice::Finish);
    let _ = voice_thread.join();
    publish(&signal(SignalKind::Listen {
        active: false,
        audio_envelope: 0.0,
    }));
    publish(&signal(SignalKind::Think {
        active: false,
        trace_id: format!("voz-{turn_number}"),
        working_set: BTreeMap::new(),
    }));
    result
}

/// The answer being given to one utterance.
struct Answering {
    id: String,
    heard: String,
    budget: SpeechBudget,
    steps: usize,
    results: Vec<(String, String)>,
}

/// Moves the answer along: what the model says goes to the voice, what it proposes is
/// carried out, and when a step ends with proposals, the next step is asked for.
#[allow(clippy::too_many_arguments)]
fn advance<S: TurnSource>(
    session: &mut SpeechSession<NemotronCognitive<S>>,
    turn: &mut Option<Answering>,
    setup: &Setup,
    act: &mut dyn FnMut(&SpeechToolIntentCandidate) -> String,
    context: &mut dyn FnMut() -> String,
    to_voice: &Sender<ToVoice>,
    generation: &AtomicU64,
    speaking: &mut usize,
    report: &mut dyn FnMut(Moment),
) -> Result<(), String> {
    let Some(answering) = turn.as_mut() else {
        return Ok(());
    };
    for event in session.pump() {
        match event {
            SessionEvent::Speak(segment) => {
                *speaking += 1;
                let features = semantic_features(&segment.text);
                let _ = to_voice.send(ToVoice::Say {
                    generation: generation.load(Ordering::SeqCst),
                    sequence: segment.sequence,
                    text: segment.text,
                    semantic_features: features,
                });
            }
            SessionEvent::ToolProposal(intent) => {
                report(Moment::Proposed {
                    tool: intent.tool_name.clone(),
                    arguments: intent.raw_arguments.clone(),
                });
                let outcome = act(&intent);
                report(Moment::Did {
                    report: outcome.clone(),
                });
                answering
                    .results
                    .push((intent.correlation_id.clone(), outcome));
            }
            SessionEvent::Withheld { .. } => {}
            SessionEvent::Alert(message) => report(Moment::Alert(message)),
            SessionEvent::Finished => {
                if session.cognitive().awaiting_results() && answering.steps < setup.steps {
                    answering.steps += 1;
                    let results = std::mem::take(&mut answering.results);
                    let budget = answering.budget;
                    let id = answering.id.clone();
                    let facts = context();
                    let cognitive = session.cognitive_mut();
                    cognitive.set_context(facts);
                    cognitive
                        .follow_up(id, &results, budget)
                        .map_err(|error| error.to_string())?;
                } else {
                    // Remembered as written for the voice; an interrupted turn ends before
                    // this and is not remembered at all.
                    let heard_from_archie = session.written_text();
                    let heard = answering.heard.clone();
                    session.cognitive_mut().remember(&heard, &heard_from_archie);
                    archie_orb::publish(&archie_orb::signal(SignalKind::Think {
                        active: false,
                        trace_id: answering.id.clone(),
                        working_set: BTreeMap::new(),
                    }));
                    *turn = None;
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn pump_voice<S: TurnSource>(
    from_voice: &Receiver<FromVoice>,
    session: &mut SpeechSession<NemotronCognitive<S>>,
    speaking: &mut usize,
    last_spoke: &mut Option<Instant>,
    report: &mut dyn FnMut(Moment),
) -> Result<(), String> {
    loop {
        match from_voice.try_recv() {
            Ok(FromVoice::Spoke {
                sequence,
                text,
                first_audio_ms,
                audio_s,
                stopped,
            }) => {
                *speaking = speaking.saturating_sub(1);
                *last_spoke = Some(Instant::now());
                if !stopped {
                    // Only what was said to the end counts as heard.
                    let _ = session.spoke(sequence);
                }
                report(Moment::Said {
                    text,
                    first_audio_ms,
                    audio_s,
                    stopped,
                });
            }
            Ok(FromVoice::Began { text }) => report(Moment::Speaking { text }),
            Ok(FromVoice::Idle) => {
                *speaking = 0;
            }
            Ok(FromVoice::Failed(message)) => {
                *speaking = speaking.saturating_sub(1);
                report(Moment::Alert(message));
            }
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => return Err("the voice stopped".into()),
        }
    }
}

/// The voice's thread: says each piece as it arrives, in order, and says when it is done.
fn speak(
    mut voice: Box<dyn Speaks>,
    mut saying: Saying,
    inbox: Receiver<ToVoice>,
    outbox: Sender<FromVoice>,
    current: Arc<AtomicU64>,
) {
    while let Ok(message) = inbox.recv() {
        match message {
            ToVoice::Say {
                generation,
                sequence,
                text,
                semantic_features,
            } => {
                if current.load(Ordering::SeqCst) != generation {
                    let _ = outbox.send(FromVoice::Spoke {
                        sequence,
                        text,
                        first_audio_ms: 0.0,
                        audio_s: 0.0,
                        stopped: true,
                    });
                    continue;
                }
                let _ = outbox.send(FromVoice::Began { text: text.clone() });
                let mut failed = None;
                let turn_id = format!("voz-{generation}-{sequence}");
                let mut last_signal = Instant::now() - Duration::from_secs(1);
                let mut published_active = false;
                let result = voice.say(&text, &mut |pcm| {
                    if current.load(Ordering::SeqCst) != generation {
                        return false;
                    }
                    if let Err(error) = saying.play(pcm) {
                        failed = Some(error.to_string());
                        return false;
                    }
                    if !published_active || last_signal.elapsed() >= Duration::from_millis(100) {
                        let envelope = if pcm.is_empty() {
                            0.0
                        } else {
                            (pcm.iter()
                                .map(|sample| {
                                    let value = *sample as f32 / i16::MAX as f32;
                                    value * value
                                })
                                .sum::<f32>()
                                / pcm.len() as f32)
                                .sqrt()
                                .clamp(0.0, 1.0)
                        };
                        publish(&signal(SignalKind::Speak {
                            active: true,
                            turn_id: turn_id.clone(),
                            semantic_features: semantic_features.clone(),
                            audio_envelope: envelope,
                        }));
                        last_signal = Instant::now();
                        published_active = true;
                    }
                    true
                });
                if published_active {
                    publish(&signal(SignalKind::Speak {
                        active: false,
                        turn_id,
                        semantic_features: Vec::new(),
                        audio_envelope: 0.0,
                    }));
                }
                match (result, failed) {
                    (_, Some(error)) => {
                        let _ = outbox.send(FromVoice::Failed(error));
                    }
                    (Ok(said), None) => {
                        if said.stopped {
                            saying.hush();
                        }
                        let _ = outbox.send(FromVoice::Spoke {
                            sequence,
                            text,
                            first_audio_ms: said.first_audio_ms,
                            audio_s: said.audio_s,
                            stopped: said.stopped,
                        });
                    }
                    (Err(error), None) => {
                        let _ = outbox.send(FromVoice::Failed(error.to_string()));
                    }
                }
            }
            ToVoice::Hush => {
                saying.hush();
                let _ = outbox.send(FromVoice::Idle);
            }
            ToVoice::Finish => break,
        }
    }
    saying.settle();
}

/// What the ears wrote, as a sentence: a run of punctuation — the recogniser adds full
/// stops through a long silence — becomes its first mark.
pub fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_mark = false;
    for c in text.trim().chars() {
        let mark = matches!(c, '.' | '?' | '!' | ',' | ';' | ':');
        if mark && last_was_mark {
            continue;
        }
        last_was_mark = mark;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_punctuation_through_silence_becomes_one_mark() {
        assert_eq!(
            tidy("Oye Archi, ¿qué hora es?.............."),
            "Oye Archi, ¿qué hora es?"
        );
        assert_eq!(tidy("  hola.  "), "hola.");
    }
}
