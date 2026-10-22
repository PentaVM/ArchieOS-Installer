//! Answers read from the social state, against the simulator's labels.
//!
//! The first test needs nothing but the generator: every question about what someone knows or
//! believes must be answered from the state, and exactly as the simulator labels it. The second
//! (ignored) measures a real checkpoint on development questions, alone and with the state
//! answering first:
//!
//! ```text
//! WANDA_DEV=<dir with dev-know.jsonl, dev-expect.jsonl, dev-belief.jsonl> \
//! WANDA_LNCM=<checkpoint dir> \
//! cargo test --release -p archie-wanda-lncm --test exact -- --ignored --nocapture
//! ```

use archie_wanda_lncm::data::{Choice, Example};
use archie_wanda_lncm::exact;
use archie_wanda_lncm::model::Lncm;
use archie_wanda_lncm::synth::{generate_with_states, SynthConfig};
use archie_wanda_lncm::Task;
use candle_core::Device;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// "know", "belief" or "expect", from an id like `synth-20261020-0-es:decide:know:1:Olivia`.
fn kind(e: &Example) -> &str {
    e.id.split(':').nth(2).unwrap_or("")
}

fn world(e: &Example) -> &str {
    e.id.split(':').next().unwrap_or("")
}

fn gold(e: &Example) -> Option<Option<usize>> {
    e.target.choice.map(|c| match c {
        Choice::Option(i) => Some(i),
        Choice::Abstain => None,
    })
}

fn from_state(
    states: &BTreeMap<String, archie_wanda_lncm::SocialState>,
    e: &Example,
) -> Option<Option<usize>> {
    let question = e.question.as_deref()?;
    let found = exact::resolve(&states[world(e)], question)?;
    Some(found.choose(e.lang, &e.options)?.choice)
}

#[test]
fn the_state_answers_knowledge_and_belief_as_the_simulator_labels_it() {
    let config = SynthConfig {
        worlds: 16,
        ..SynthConfig::default()
    };
    let (out, states) = generate_with_states(&config).unwrap();
    // Per kind: questions, answered from the state, answered as labelled.
    let mut counts: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut missed = Vec::new();
    for e in out.examples.iter().filter(|e| e.task == Task::Decide) {
        let c = counts.entry(kind(e).to_string()).or_default();
        c.0 += 1;
        match from_state(&states, e) {
            Some(choice) => {
                c.1 += 1;
                if Some(choice) == gold(e) {
                    c.2 += 1;
                }
            }
            None if kind(e) != "expect" => {
                missed.push(format!("{} | {:?}", e.id, e.question));
            }
            None => {}
        }
    }
    eprintln!("(questions, answered, right) {counts:?}");
    assert!(
        missed.is_empty(),
        "not answered from the state:\n{}",
        missed.join("\n")
    );
    for k in ["know", "belief"] {
        let (n, answered, right) = counts[k];
        assert!(n > 20, "{k}: {n} questions");
        assert_eq!(
            answered, n,
            "{k}: every question is answered from the state"
        );
        assert_eq!(right, n, "{k}: every answer is the simulator's");
    }
    let (n, answered, _) = counts["expect"];
    assert!(
        answered * 2 > n,
        "most expectation questions name one the state holds"
    );
}

#[test]
#[ignore = "needs WANDA_DEV and WANDA_LNCM"]
fn development_questions_with_and_without_the_state() {
    let dev = PathBuf::from(std::env::var("WANDA_DEV").expect("WANDA_DEV"));
    let dir = PathBuf::from(std::env::var("WANDA_LNCM").expect("WANDA_LNCM"));
    let model = Lncm::load(&dir, &Device::Cpu).unwrap();
    let mut wanted: Vec<Example> = Vec::new();
    for k in ["know", "expect", "belief"] {
        let text = std::fs::read_to_string(dev.join(format!("dev-{k}.jsonl"))).unwrap();
        wanted.extend(text.lines().map(|l| serde_json::from_str(l).unwrap()));
    }
    // World ids are `synth-<seed>-<index>-<lang>`: regenerate up to the last index needed.
    let parts: Vec<(u64, usize)> = wanted
        .iter()
        .map(|e| {
            let p: Vec<&str> = world(e).split('-').collect();
            (p[1].parse().unwrap(), p[2].parse().unwrap())
        })
        .collect();
    let seed = parts[0].0;
    assert!(parts.iter().all(|(s, _)| *s == seed), "one seed");
    let config = SynthConfig {
        seed,
        worlds: parts.iter().map(|(_, i)| i + 1).max().unwrap(),
        ..SynthConfig::default()
    };
    let (out, states) = generate_with_states(&config).unwrap();
    let regenerated: BTreeMap<&str, &Example> =
        out.examples.iter().map(|e| (e.id.as_str(), e)).collect();

    // Per kind: questions, model right, answered from the state, state right, both right.
    let mut tally: BTreeMap<String, [usize; 5]> = BTreeMap::new();
    for e in &wanted {
        let again = regenerated[e.id.as_str()];
        assert_eq!(
            (&again.question, &again.state, &again.options),
            (&e.question, &e.state, &e.options),
            "{} is the same question it was",
            e.id
        );
        let question = e.question.as_deref().unwrap();
        let by_model = model
            .decide(e.lang, question, e.state.as_deref(), None, &e.options)
            .unwrap()
            .choice;
        let by_state = from_state(&states, e);
        let both = by_state.unwrap_or(by_model);
        let t = tally.entry(kind(e).to_string()).or_default();
        t[0] += 1;
        t[1] += usize::from(Some(by_model) == gold(e));
        t[2] += usize::from(by_state.is_some());
        t[3] += usize::from(by_state.is_some() && by_state == gold(e));
        t[4] += usize::from(Some(both) == gold(e));
    }
    let share = |a: usize, n: usize| a as f64 / n.max(1) as f64;
    println!("kind     n  model  answered  state_right  state+model");
    for (k, [n, model_right, answered, state_right, both_right]) in &tally {
        println!(
            "{k:<7}{n:>4}  {:.3}  {:>8}  {:>11}  {:.3}",
            share(*model_right, *n),
            answered,
            state_right,
            share(*both_right, *n),
        );
    }
}
