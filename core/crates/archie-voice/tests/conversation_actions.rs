//! Real conversation loop, deterministic ASR/model/audio boundaries; no model load.
use archie_model_runtime::{StreamEvent, ToolCall, ToolChatMessage, ToolChatRequest};
use archie_speech::{FloorPolicy, SessionPlan, SpeechCapabilities, SpeechSession};
use archie_voice::actions::{ActionDispatcher, AsyncActions};
use archie_voice::asr::{AsrStream, Heard, SpeechRecognizer};
use archie_voice::conversation::{converse_with_actions, Hearing, Moment, Saying, Setup};
use archie_voice::nemotron::{NemotronCognitive, TurnSource, TurnStream, Update};
use archie_voice::voice::{Said, Speaks};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

struct Ears;
struct EarsStream {
    last: u32,
    heard: VecDeque<Heard>,
}
impl SpeechRecognizer for Ears {
    fn listen(&self, _: &str, _: &[&str], _: f32) -> Result<Box<dyn AsrStream + '_>, String> {
        Ok(Box::new(EarsStream {
            last: 0,
            heard: VecDeque::new(),
        }))
    }
    fn recognize(&self, _: &[f32], _: u32) -> Result<String, String> {
        unreachable!()
    }
}
impl AsrStream for EarsStream {
    fn push(&mut self, samples: &[f32], _: u32) -> Result<(), String> {
        let tag = samples.iter().copied().fold(0.0_f32, f32::max);
        if tag > 0.0 && tag.to_bits() != self.last {
            self.last = tag.to_bits();
            self.heard.push_back(Heard {
                text: if tag < 0.15 {
                    "Archie, mira la pantalla."
                } else {
                    "Archie, no, dime la hora."
                }
                .into(),
                is_final: true,
            });
        }
        Ok(())
    }
    fn next(&mut self) -> Result<Option<Heard>, String> {
        Ok(self.heard.pop_front())
    }
    fn end_utterance(&mut self) -> Result<(), String> {
        Ok(())
    }
}
struct Model(Arc<AtomicUsize>);
impl TurnSource for Model {
    fn start(&mut self, _: ToolChatRequest) -> TurnStream {
        let first = self.0.fetch_add(1, Ordering::SeqCst) == 0;
        let (send, updates) = mpsc::channel();
        let event = if first {
            StreamEvent::ToolCall(ToolCall {
                id: "look0".into(),
                name: "look".into(),
                arguments: "{}".into(),
            })
        } else {
            StreamEvent::Text("Listo. ".into())
        };
        send.send(Update::Event(event)).unwrap();
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
struct Voice;
impl Speaks for Voice {
    fn sample_rate(&self) -> u32 {
        16_000
    }
    fn say(&mut self, _: &str, hear: &mut dyn FnMut(&[i16]) -> bool) -> Result<Said, String> {
        let stopped = !hear(&[0; 320]);
        Ok(Said {
            sample_rate: 16_000,
            audio_s: 0.02,
            elapsed_s: 0.0,
            first_audio_ms: 0.0,
            stopped,
        })
    }
}

#[test]
fn a_slow_tool_does_not_block_hearing_and_a_new_named_turn_cancels_it() {
    correction_during_tool(None, "streamed");
}

struct Rehear;
impl SpeechRecognizer for Rehear {
    fn listen(&self, _: &str, _: &[&str], _: f32) -> Result<Box<dyn AsrStream + '_>, String> {
        unreachable!()
    }
    fn recognize(&self, samples: &[f32], _: u32) -> Result<String, String> {
        Ok(if samples.iter().any(|sample| *sample > 0.15) {
            "No, dime la hora."
        } else {
            "Mira la pantalla."
        }
        .into())
    }
}

#[test]
fn streamed_name_still_cancels_when_second_recognizer_omits_it() {
    correction_during_tool(Some(&Rehear), "reheard");
}

fn correction_during_tool(rehear: Option<&dyn SpeechRecognizer>, label: &str) {
    correction_with_ears(&Ears, Setup::default(), rehear, label);
}

struct NoName;
struct NoNameStream<'a>(Box<dyn AsrStream + 'a>);
impl SpeechRecognizer for NoName {
    fn listen(
        &self,
        language: &str,
        phrases: &[&str],
        boost: f32,
    ) -> Result<Box<dyn AsrStream + '_>, String> {
        Ok(Box::new(NoNameStream(
            Ears.listen(language, phrases, boost)?,
        )))
    }
    fn recognize(&self, _: &[f32], _: u32) -> Result<String, String> {
        unreachable!()
    }
}
impl AsrStream for NoNameStream<'_> {
    fn push(&mut self, samples: &[f32], rate: u32) -> Result<(), String> {
        self.0.push(samples, rate)
    }
    fn next(&mut self) -> Result<Option<Heard>, String> {
        Ok(self.0.next()?.map(|mut heard| {
            heard.text = heard.text.replace("Archie, ", "");
            heard
        }))
    }
    fn end_utterance(&mut self) -> Result<(), String> {
        self.0.end_utterance()
    }
}

#[test]
fn explicit_solo_mode_can_supersede_a_tool_without_a_recognized_name() {
    correction_with_ears(
        &NoName,
        Setup {
            always: true,
            ..Default::default()
        },
        None,
        "solo",
    );
}

fn correction_with_ears(
    ears: &dyn SpeechRecognizer,
    setup: Setup,
    rehear: Option<&dyn SpeechRecognizer>,
    label: &str,
) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let completed_tool = Arc::new(AtomicBool::new(false));
    let completed = completed_tool.clone();
    let (input, audio) = mpsc::channel();
    input.send(vec![0.1; 2560]).unwrap();
    input.send(vec![0.0; 6400]).unwrap();
    let next_audio = input.clone();
    let mut actions = AsyncActions::new(
        runtime.handle().clone(),
        Duration::from_secs(1),
        move |_| {
            next_audio.send(vec![0.2; 2560]).unwrap();
            next_audio.send(vec![0.0; 6400]).unwrap();
            let completed = completed.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(800)).await;
                completed.store(true, Ordering::SeqCst);
                "old result".into()
            }
        },
    );
    let plan = SessionPlan {
        language: "es".into(),
        listener: SpeechCapabilities {
            partial_transcript: true,
            languages: vec!["es".into()],
            ..Default::default()
        },
        voice: SpeechCapabilities {
            accepts_external_response_text: true,
            renders_external_response_verbatim: true,
            languages: vec!["es".into()],
            ..Default::default()
        },
    };
    let mut session = SpeechSession::new(
        plan,
        FloorPolicy::default(),
        NemotronCognitive::new(Model(calls.clone()), "test", vec!["es".into()], vec![]),
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let watchdog_stop = stop.clone();
    let (done, waiting) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if waiting.recv_timeout(Duration::from_secs(3)).is_err() {
            watchdog_stop.store(true, Ordering::SeqCst);
        }
    });
    let path =
        std::env::temp_dir().join(format!("archie-actions-{}-{label}.wav", std::process::id()));
    let mut moments = Vec::new();
    let result = converse_with_actions(
        &setup,
        ears,
        rehear,
        Box::new(Voice),
        Hearing::Stream(audio),
        Saying::Recording {
            path: path.clone(),
            samples: vec![],
            rate: 16_000,
        },
        &mut session,
        &mut actions,
        &mut String::new,
        stop.clone(),
        &mut |moment| {
            if matches!(moment, Moment::Ready) && calls.load(Ordering::SeqCst) >= 2 {
                stop.store(true, Ordering::SeqCst);
            }
            moments.push(moment);
        },
    );
    let _ = done.send(());
    watchdog.join().unwrap();
    let _ = std::fs::remove_file(path);
    result.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the correction must be answered while the old tool waits"
    );
    assert!(
        !completed_tool.load(Ordering::SeqCst),
        "the superseded tool future must be cancelled"
    );
    assert!(!moments
        .iter()
        .any(|m| matches!(m, Moment::Did { report } if report == "old result")));
    assert!(moments.contains(&Moment::Interrupted), "{moments:?}");
    assert!(moments.iter().any(
        |m| matches!(m, Moment::Timing { turn_id, phase: "tool_started", .. } if turn_id == "voz-1")
    ));
    assert!(moments.iter().any(|m| matches!(m, Moment::Timing { turn_id, phase: "first_segment", queued_segments, .. } if turn_id == "voz-2" && *queued_segments <= 2)));
}

struct TwoTools(Arc<AtomicUsize>);
impl TurnSource for TwoTools {
    fn start(&mut self, request: ToolChatRequest) -> TurnStream {
        let step = self.0.fetch_add(1, Ordering::SeqCst);
        let (send, updates) = mpsc::channel();
        if step == 0 {
            for id in ["one", "two"] {
                send.send(Update::Event(StreamEvent::ToolCall(ToolCall {
                    id: id.into(),
                    name: id.into(),
                    arguments: "{}".into(),
                })))
                .unwrap();
            }
        } else {
            let results: Vec<_> = request
                .messages
                .iter()
                .filter_map(|message| match message {
                    ToolChatMessage::Tool { call_id, content } => {
                        Some((call_id.as_str(), content.as_str()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(results, vec![("one", "done-one"), ("two", "done-two")]);
            send.send(Update::Event(StreamEvent::Text("Listo. ".into())))
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
fn multiple_async_results_reach_the_model_once_in_proposal_order() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let count = started.clone();
    let mut actions = AsyncActions::new(
        runtime.handle().clone(),
        Duration::from_secs(1),
        move |intent: archie_speech::SpeechToolIntentCandidate| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                format!("done-{}", intent.tool_name)
            }
        },
    );
    let moments = run_one(TwoTools(calls.clone()), &mut actions, "multiple");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(
        moments
            .iter()
            .filter(|m| matches!(m, Moment::Did { .. }))
            .count(),
        2
    );
    assert!(moments.iter().any(|m| matches!(
        m,
        Moment::Timing {
            phase: "complete",
            ..
        }
    )));
}

struct Failed;
impl TurnSource for Failed {
    fn start(&mut self, _: ToolChatRequest) -> TurnStream {
        let (send, updates) = mpsc::channel();
        send.send(Update::Failed("backend unavailable".into()))
            .unwrap();
        TurnStream {
            updates,
            stop: Box::new(|| {}),
        }
    }
}

#[test]
fn backend_failure_finishes_the_real_loop_without_waiting_for_shutdown() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut actions = AsyncActions::new(
        runtime.handle().clone(),
        Duration::from_secs(1),
        |_| async { panic!("no action was proposed") },
    );
    let moments = run_one(Failed, &mut actions, "failed");
    assert!(moments.iter().any(|m| matches!(
        m,
        Moment::Timing {
            phase: "failed",
            ..
        }
    )));
    assert!(!moments.iter().any(|m| matches!(m, Moment::Said { .. })));
}

fn run_one<S: TurnSource>(
    source: S,
    actions: &mut dyn ActionDispatcher,
    label: &str,
) -> Vec<Moment> {
    let plan = SessionPlan {
        language: "es".into(),
        listener: SpeechCapabilities {
            partial_transcript: true,
            languages: vec!["es".into()],
            ..Default::default()
        },
        voice: SpeechCapabilities {
            accepts_external_response_text: true,
            renders_external_response_verbatim: true,
            languages: vec!["es".into()],
            ..Default::default()
        },
    };
    let mut session = SpeechSession::new(
        plan,
        FloorPolicy::default(),
        NemotronCognitive::new(source, "test", vec!["es".into()], vec![]),
    )
    .unwrap();
    let (input, audio) = mpsc::channel();
    input.send(vec![0.1; 2560]).unwrap();
    input.send(vec![0.0; 6400]).unwrap();
    drop(input);
    let stop = Arc::new(AtomicBool::new(false));
    let watchdog_stop = stop.clone();
    let (done, waiting) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if waiting.recv_timeout(Duration::from_secs(5)).is_err() {
            watchdog_stop.store(true, Ordering::SeqCst);
        }
    });
    let path =
        std::env::temp_dir().join(format!("archie-actions-{}-{label}.wav", std::process::id()));
    let mut moments = vec![];
    let result = converse_with_actions(
        &Setup::default(),
        &Ears,
        None,
        Box::new(Voice),
        Hearing::Stream(audio),
        Saying::Recording {
            path: path.clone(),
            samples: vec![],
            rate: 16_000,
        },
        &mut session,
        actions,
        &mut String::new,
        stop.clone(),
        &mut |moment| moments.push(moment),
    );
    let _ = done.send(());
    watchdog.join().unwrap();
    let _ = std::fs::remove_file(path);
    result.unwrap();
    assert!(
        !stop.load(Ordering::SeqCst),
        "the loop did not finish before the watchdog"
    );
    moments
}
