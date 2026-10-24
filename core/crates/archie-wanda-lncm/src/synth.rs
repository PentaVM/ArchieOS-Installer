//! Owned synthetic social worlds, in Spanish and English, with exact labels.
//!
//! A world is one group conversation. People join and leave; facts are stated and changed while
//! some are absent; someone tells an absent person what changed; people behave according to
//! hidden traits (late or punctual, reliable or not, direct or evasive, conflict-avoidant or
//! not, slow or quick to reply), consistently but not always; moods show in what they write.
//!
//! Every label comes from the simulation, never from a model:
//! - the delta lines of every chunk (so the reader's update task has exact targets);
//! - who knows and believes what (from presence, computed by the same bookkeeping the runtime
//!   uses);
//! - what to expect of each person (from the hidden trait, asked only when it showed at least
//!   twice; otherwise the right answer is "cannot know").
//!
//! Composed answers here are templates grounded in the exact state: a starting point and a
//! fallback. The distillation pipeline replaces them with teachers' answers to the same cases.

use crate::data::{split_for, Case, Choice, Example, Provenance, Split, Target};
use crate::reader::{read_recording, ReaderConfig, Turn, TurnKind};
use crate::state::SocialState;
use crate::{Lang, Result, Task};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::collections::BTreeMap;

pub const GENERATOR: &str = "archie-wanda-lncm.synth.v1";
const START_MS: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

#[derive(Debug, Clone)]
pub struct SynthConfig {
    pub seed: u64,
    pub worlds: usize,
    pub langs: Vec<Lang>,
    pub min_turns: usize,
    pub max_turns: usize,
    pub dev_share: f32,
    pub reader: ReaderConfig,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            seed: 20_261_020,
            worlds: 200,
            langs: vec![Lang::Es, Lang::En],
            min_turns: 60,
            max_turns: 240,
            dev_share: 0.1,
            reader: ReaderConfig::default(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SynthOutput {
    pub examples: Vec<Example>,
    pub cases: Vec<Case>,
}

struct TraitSpec {
    key: &'static str,
    statement: [&'static str; 2],
    yes: [&'static [&'static str]; 2],
    no: [&'static [&'static str]; 2],
    question: [&'static str; 2],
}

const TRAITS: &[TraitSpec] = &[
    TraitSpec {
        key: "late",
        statement: ["suele llegar tarde", "is usually late"],
        yes: [
            &[
                "Perdón, voy {n} minutos tarde.",
                "Me he retrasado otra vez, llego en {n}.",
                "Llego tarde, empezad sin mí.",
            ],
            &[
                "Sorry, running {n} minutes late.",
                "Late again, there in {n}.",
                "Running late, start without me.",
            ],
        ],
        no: [
            &[
                "Ya estoy aquí, he llegado pronto.",
                "Llegué puntual, os espero dentro.",
            ],
            &[
                "I'm here already, came early.",
                "Arrived on time, waiting inside.",
            ],
        ],
        question: [
            "Si quedas con {X} a las ocho, ¿es probable que llegue tarde?",
            "If you meet {X} at eight, are they likely to be late?",
        ],
    },
    TraitSpec {
        key: "reliable",
        statement: ["cumple lo que promete", "keeps their word"],
        yes: [
            &[
                "Hecho, ya lo envié como dije.",
                "Listo, lo que prometí ya está.",
            ],
            &[
                "Done, sent it like I said.",
                "Sorted, what I promised is done.",
            ],
        ],
        no: [
            &[
                "Al final no pude hacerlo, lo siento.",
                "Se me olvidó lo que quedé en hacer, perdón.",
            ],
            &[
                "In the end I couldn't do it, sorry.",
                "I forgot what I said I'd do, sorry.",
            ],
        ],
        question: [
            "Si {X} promete encargarse de algo, ¿lo cumplirá?",
            "If {X} promises to handle something, will they follow through?",
        ],
    },
    TraitSpec {
        key: "direct",
        statement: ["dice las cosas claras", "says things plainly"],
        yes: [
            &[
                "Te lo digo claro: no me convence.",
                "Sin rodeos: así no funciona.",
            ],
            &[
                "I'll be straight: I don't like it.",
                "No beating around the bush: this won't work.",
            ],
        ],
        no: [
            &[
                "Bueno... no sé, quizá, ya veremos.",
                "Mmm, puede ser, como veáis.",
            ],
            &[
                "Well... I don't know, maybe, we'll see.",
                "Hmm, could be, whatever you think.",
            ],
        ],
        question: [
            "Si a {X} no le gusta un plan, ¿te lo dirá directamente?",
            "If {X} dislikes a plan, will they tell you directly?",
        ],
    },
    TraitSpec {
        key: "avoids-conflict",
        statement: ["evita los conflictos", "avoids conflict"],
        yes: [
            &[
                "Mejor no discutamos, lo que digáis está bien.",
                "Prefiero no meterme en esto.",
            ],
            &[
                "Let's not argue, whatever you say is fine.",
                "I'd rather stay out of this.",
            ],
        ],
        no: [
            &[
                "No estoy de acuerdo y lo voy a decir.",
                "Esto hay que discutirlo, no me callo.",
            ],
            &[
                "I disagree and I'm going to say so.",
                "We need to argue this out, I won't keep quiet.",
            ],
        ],
        question: [
            "Si hay un desacuerdo en el grupo, ¿{X} intentará evitar la discusión?",
            "If the group disagrees, will {X} try to avoid the argument?",
        ],
    },
    TraitSpec {
        key: "slow-replies",
        statement: ["tarda en responder", "is slow to reply"],
        yes: [
            &[
                "Perdón, acabo de ver esto.",
                "Uf, lo vi tarde, ¿sigue en pie?",
            ],
            &["Sorry, just saw this.", "Oops, saw this late, still on?"],
        ],
        no: [
            &[
                "¡Aquí estoy, respondo al momento!",
                "Visto, te contesto ya.",
            ],
            &["Here, replying right away!", "Seen, answering now."],
        ],
        question: [
            "Si le escribes a {X} algo urgente, ¿tardará en responder?",
            "If you text {X} something urgent, will they take long to reply?",
        ],
    },
];

struct AffectSpec {
    label: [&'static str; 2],
    intensity: u8,
    text: [&'static [&'static str]; 2],
}

const AFFECTS: &[AffectSpec] = &[
    AffectSpec {
        label: ["molesto", "annoyed"],
        intensity: 70,
        text: [
            &[
                "Estoy bastante molesto con cómo se hizo esto.",
                "Me ha sentado fatal, la verdad.",
            ],
            &[
                "I'm pretty annoyed with how this was done.",
                "Honestly, that really upset me.",
            ],
        ],
    },
    AffectSpec {
        label: ["contento", "happy"],
        intensity: 80,
        text: [
            &["¡Qué ilusión, me encanta!", "Estoy feliz con cómo salió."],
            &[
                "So excited, I love it!",
                "I'm happy with how it turned out.",
            ],
        ],
    },
    AffectSpec {
        label: ["cansado", "tired"],
        intensity: 60,
        text: [
            &["Estoy agotado esta semana.", "No puedo más, qué semana."],
            &["I'm exhausted this week.", "I can't anymore, what a week."],
        ],
    },
    AffectSpec {
        label: ["preocupado", "worried"],
        intensity: 65,
        text: [
            &[
                "Me preocupa que no lleguemos a tiempo.",
                "Estoy un poco preocupado por esto.",
            ],
            &[
                "I'm worried we won't make it in time.",
                "I'm a bit worried about this.",
            ],
        ],
    },
];

struct TopicSpec {
    topic: [&'static str; 2],
    verb: [&'static str; 2],
    values: [&'static [&'static str]; 2],
}

const TOPICS: &[TopicSpec] = &[
    TopicSpec {
        topic: ["el viaje", "the trip"],
        verb: ["es en {v}", "is in {v}"],
        values: [
            &["mayo", "junio", "julio", "agosto", "septiembre"],
            &["May", "June", "July", "August", "September"],
        ],
    },
    TopicSpec {
        topic: ["el presupuesto", "the budget"],
        verb: ["es de {v}", "is {v}"],
        values: [
            &["800 euros", "950 euros", "1.200 euros", "1.500 euros"],
            &["800 euros", "950 euros", "1,200 euros", "1,500 euros"],
        ],
    },
    TopicSpec {
        topic: ["el punto de encuentro", "the meeting point"],
        verb: ["es {v}", "is {v}"],
        values: [
            &[
                "la estación",
                "la azotea",
                "la vieja biblioteca",
                "el gimnasio",
            ],
            &["the station", "the rooftop", "the old library", "the gym"],
        ],
    },
    TopicSpec {
        topic: ["la cena", "the dinner"],
        verb: ["es el {v}", "is on {v}"],
        values: [
            &["lunes", "martes", "jueves", "viernes"],
            &["Monday", "Tuesday", "Thursday", "Friday"],
        ],
    },
    TopicSpec {
        topic: ["la entrega", "the deadline"],
        verb: ["es el día {v}", "is the {v}"],
        values: [&["3", "10", "15", "21"], &["3rd", "10th", "15th", "21st"]],
    },
];

const NAMES: [&[&str]; 2] = [
    &[
        "Alba",
        "Bernardo",
        "Celia",
        "Darío",
        "Emma",
        "Fausto",
        "Gala",
        "Héctor",
        "Irene",
        "Julián",
        "Karen",
        "Lucas",
        "Marina",
        "Noel",
        "Olivia",
        "Pedro",
        "Raquel",
        "Sergio",
        "Tania",
        "Ulises",
        "Valeria",
        "Ximena",
        "Yago",
        "Zoe",
        "Ana María",
        "José Luis",
        "Nuria",
        "Íñigo",
        "Begoña",
        "Rocío",
    ],
    &[
        "Alice", "Bruno", "Carla", "Diego", "Elena", "Farid", "Greta", "Hugo", "Ines", "Jonah",
        "Kira", "Leo", "Maya", "Nico", "Olga", "Pablo", "Rosa", "Samir", "Tara", "Victor", "Wendy",
        "Xavier", "Yusuf", "Zara", "Mary Ann", "Liam", "Priya", "Chen", "Amara", "Noah",
    ],
];

const CHATTER: [&[&str]; 2] = [
    &[
        "¿Alguien vio el partido anoche?",
        "Otra vez atasco, qué día.",
        "Qué buena foto.",
        "¿Me pasáis la lista de canciones?",
        "Café primero, luego pienso.",
        "¿Quién se llevó mi cargador?",
        "La panadería nueva está muy bien.",
        "Jaja, clásico.",
        "De acuerdo.",
        "Lo miro y te digo.",
        "¡Gracias!",
        "Llueve toda la semana, dicen.",
        "Ya arreglé la bici por fin.",
    ],
    &[
        "Did anyone watch the match last night?",
        "Traffic again, what a day.",
        "Great photo.",
        "Can someone send the playlist?",
        "Coffee first, then I can think.",
        "Who took my charger?",
        "The new bakery is really good.",
        "Ha, classic.",
        "Agreed.",
        "Let me check and get back to you.",
        "Thanks!",
        "Rain all week, apparently.",
        "Finally fixed the bike.",
    ],
];

/// Questions that are not for Wanda, with whether they are yes/no questions.
const NOT_SOCIAL: [&[(&str, bool)]; 2] = [
    &[
        ("¿Qué tiempo hará mañana en Madrid?", false),
        ("Abre Blender y crea un cubo.", false),
        ("¿Cuánto es 17 por 23?", false),
        ("¿Está encendido el router?", true),
        ("Pon música tranquila.", false),
        ("¿Hay actualizaciones del sistema?", true),
        ("Resume este documento.", false),
        ("¿Cuál es la capital de Australia?", false),
        ("¿Tengo batería para dos horas?", true),
        ("Busca vuelos a Lisboa en julio.", false),
        ("¿Funciona la cámara del salón?", true),
        ("Traduce esto al inglés.", false),
        ("¿Qué hora es en Tokio?", false),
        ("Edita el vídeo y quita los silencios.", false),
        ("¿Está guardado el archivo?", true),
    ],
    &[
        ("What's the weather in Madrid tomorrow?", false),
        ("Open Blender and make a cube.", false),
        ("What is 17 times 23?", false),
        ("Is the router on?", true),
        ("Play some calm music.", false),
        ("Are there system updates?", true),
        ("Summarise this document.", false),
        ("What is the capital of Australia?", false),
        ("Do I have battery for two hours?", true),
        ("Find flights to Lisbon in July.", false),
        ("Is the living room camera working?", true),
        ("Translate this into Spanish.", false),
        ("What time is it in Tokyo?", false),
        ("Edit the video and cut the silences.", false),
        ("Is the file saved?", true),
    ],
];

fn li(lang: Lang) -> usize {
    match lang {
        Lang::Es => 0,
        Lang::En => 1,
    }
}

fn person_ref(label: &str) -> String {
    if label.contains(' ') {
        format!("\"{label}\"")
    } else {
        label.to_string()
    }
}

struct World {
    id: String,
    lang: Lang,
    owner: String,
    /// Non-owner people: label → traits (index, has).
    traits: BTreeMap<String, Vec<(usize, bool)>>,
    /// (label, trait index) → (manifested as the trait, manifested as its opposite).
    shown: BTreeMap<(String, usize), (u32, u32)>,
    topics: Vec<usize>,
    turns: Vec<Turn>,
    gold: BTreeMap<u64, Vec<String>>,
}

fn build_world(seed: u64, index: usize, lang: Lang, config: &SynthConfig) -> World {
    let mut rng = StdRng::seed_from_u64(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ index as u64);
    let l = li(lang);
    let mut names: Vec<&str> = NAMES[l].to_vec();
    names.shuffle(&mut rng);
    let others = rng.gen_range(4..=7);
    let owner = names[0].to_string();
    let people: Vec<String> = names[1..=others].iter().map(|s| s.to_string()).collect();
    let mut traits = BTreeMap::new();
    for p in &people {
        let mut picks: Vec<usize> = (0..TRAITS.len()).collect();
        picks.shuffle(&mut rng);
        let n = rng.gen_range(1..=3);
        traits.insert(
            p.clone(),
            picks[..n].iter().map(|&t| (t, rng.gen_bool(0.5))).collect(),
        );
    }
    let mut topic_ids: Vec<usize> = (0..TOPICS.len()).collect();
    topic_ids.shuffle(&mut rng);
    let topics: Vec<usize> = topic_ids[..rng.gen_range(2..=3)].to_vec();
    let chat = ["grupo", "group"][l].to_string();
    let mut world = World {
        id: format!("synth-{seed}-{index}-{}", ["es", "en"][l]),
        lang,
        owner: owner.clone(),
        traits,
        shown: BTreeMap::new(),
        topics: topics.clone(),
        turns: Vec::new(),
        gold: BTreeMap::new(),
    };
    let mut at = START_MS + rng.gen_range(0..86_400_000);
    let mut present: Vec<String> = vec![owner.clone()];
    // Someone can only be told something once they are someone the conversation knows.
    let mut known: Vec<String> = vec![owner.clone()];
    let mut turn = 0u64;
    let push = |world: &mut World, turn: &mut u64, at: i64, kind: TurnKind, gold: Vec<String>| {
        *turn += 1;
        world.turns.push(Turn {
            turn: *turn,
            at_ms: at,
            chat: chat.clone(),
            kind,
            evidence: format!("{}#{}", world.id, *turn),
            label: None,
        });
        if !gold.is_empty() {
            world.gold.insert(*turn, gold);
        }
    };
    let initial = rng.gen_range(2..=people.len().min(4));
    for p in &people[..initial] {
        present.push(p.clone());
        known.push(p.clone());
        push(
            &mut world,
            &mut turn,
            at,
            TurnKind::Join { person: p.clone() },
            vec![],
        );
    }
    let mut values: BTreeMap<usize, String> = BTreeMap::new();
    for &t in &topics {
        let spec = &TOPICS[t];
        let v = spec.values[l].choose(&mut rng).expect("values").to_string();
        values.insert(t, v.clone());
        let speaker = present.choose(&mut rng).expect("someone").clone();
        let text = capitalise(&format!(
            "{} {}.",
            spec.topic[l],
            spec.verb[l].replace("{v}", &v)
        ));
        at += rng.gen_range(30_000..600_000);
        let n = turn + 1;
        push(
            &mut world,
            &mut turn,
            at,
            TurnKind::Message { speaker, text },
            vec![format!("fact {} = {v} @{n}", spec.topic[l])],
        );
    }
    let target = rng.gen_range(config.min_turns..=config.max_turns.max(config.min_turns));
    while (turn as usize) < target {
        let roll: f64 = rng.gen();
        let absent: Vec<String> = people
            .iter()
            .filter(|p| !present.contains(p))
            .cloned()
            .collect();
        let tellable: Vec<String> = absent
            .iter()
            .filter(|p| known.contains(p))
            .cloned()
            .collect();
        let speakers: Vec<String> = present.iter().filter(|p| **p != owner).cloned().collect();
        at += rng.gen_range(20_000..900_000);
        if roll < 0.05 && speakers.len() > 2 {
            let who = speakers.choose(&mut rng).expect("speaker").clone();
            present.retain(|p| *p != who);
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Leave { person: who },
                vec![],
            );
        } else if roll < 0.10 && !absent.is_empty() {
            let who = absent.choose(&mut rng).expect("absent").clone();
            present.push(who.clone());
            if !known.contains(&who) {
                known.push(who.clone());
            }
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Join { person: who },
                vec![],
            );
        } else if roll < 0.16 {
            let t = *topics.choose(&mut rng).expect("topic");
            let spec = &TOPICS[t];
            let current = values[&t].clone();
            let options: Vec<&str> = spec.values[l]
                .iter()
                .copied()
                .filter(|v| *v != current)
                .collect();
            let v = options.choose(&mut rng).expect("another value").to_string();
            values.insert(t, v.clone());
            let speaker = present.choose(&mut rng).expect("someone").clone();
            let said = spec.verb[l].replace("{v}", &v);
            let text = match lang {
                Lang::Es => format!("Cambio de planes: ahora {} {said}.", spec.topic[l]),
                Lang::En => format!("Change of plan: {} {said} now.", spec.topic[l]),
            };
            let n = turn + 1;
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Message { speaker, text },
                vec![format!("fact {} = {v} @{n}", spec.topic[l])],
            );
        } else if roll < 0.19 && !tellable.is_empty() && !speakers.is_empty() {
            let t = *topics.choose(&mut rng).expect("topic");
            let spec = &TOPICS[t];
            let v = values[&t].clone();
            let teller = speakers.choose(&mut rng).expect("speaker").clone();
            let told = tellable.choose(&mut rng).expect("tellable").clone();
            let said = spec.verb[l].replace("{v}", &v);
            let text = match lang {
                Lang::Es => format!("Ya se lo conté a {told}: {} {said}.", spec.topic[l]),
                Lang::En => format!("I already told {told}: {} {said}.", spec.topic[l]),
            };
            let n = turn + 1;
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Message {
                    speaker: teller,
                    text,
                },
                vec![
                    format!("fact {} = {v} @{n}", spec.topic[l]),
                    format!("told {} {} = {v} @{n}", person_ref(&told), spec.topic[l]),
                ],
            );
        } else if roll < 0.33 && !speakers.is_empty() {
            let who = speakers.choose(&mut rng).expect("speaker").clone();
            let (trait_index, has) = *world.traits[&who].choose(&mut rng).expect("a trait");
            let spec = &TRAITS[trait_index];
            let as_trait = if rng.gen_bool(0.85) { has } else { !has };
            let pool = if as_trait { spec.yes[l] } else { spec.no[l] };
            let text = pool
                .choose(&mut rng)
                .expect("text")
                .replace("{n}", &rng.gen_range(5..40).to_string());
            if trait_index == 4 && as_trait {
                at += rng.gen_range(3_600_000..14_400_000); // slow to reply, in time too
            }
            let entry = world.shown.entry((who.clone(), trait_index)).or_default();
            if as_trait {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }
            let line = format!(
                "expect {} {} {}: {}",
                person_ref(&who),
                if as_trait { '+' } else { '-' },
                spec.key,
                spec.statement[l]
            );
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Message { speaker: who, text },
                vec![line],
            );
        } else if roll < 0.38 && !speakers.is_empty() {
            let who = speakers.choose(&mut rng).expect("speaker").clone();
            let affect = AFFECTS.choose(&mut rng).expect("affect");
            let text = affect.text[l].choose(&mut rng).expect("text").to_string();
            let line = format!(
                "affect {} {} {}",
                person_ref(&who),
                affect.label[l],
                affect.intensity
            );
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Message { speaker: who, text },
                vec![line],
            );
        } else {
            let who = present.choose(&mut rng).expect("someone").clone();
            let text = CHATTER[l].choose(&mut rng).expect("chatter").to_string();
            push(
                &mut world,
                &mut turn,
                at,
                TurnKind::Message { speaker: who, text },
                vec![],
            );
        }
    }
    world
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Generates worlds and turns them into examples (all four tasks) and cases for teachers.
pub fn generate(config: &SynthConfig) -> Result<SynthOutput> {
    let mut out = SynthOutput::default();
    for index in 0..config.worlds {
        for &lang in &config.langs {
            let world = build_world(config.seed, index, lang, config);
            emit(&world, config, &mut out)?;
        }
    }
    Ok(out)
}

/// [`generate`], keeping each world's final state by world id: the state its questions were
/// asked against, for checks that read it directly.
pub fn generate_with_states(
    config: &SynthConfig,
) -> Result<(SynthOutput, BTreeMap<String, SocialState>)> {
    let mut out = SynthOutput::default();
    let mut states = BTreeMap::new();
    for index in 0..config.worlds {
        for &lang in &config.langs {
            let world = build_world(config.seed, index, lang, config);
            let state = emit(&world, config, &mut out)?;
            states.insert(world.id.clone(), state);
        }
    }
    Ok((out, states))
}

/// Writes a world's examples and cases; returns its final state.
fn emit(world: &World, config: &SynthConfig, out: &mut SynthOutput) -> Result<SocialState> {
    let lang = world.lang;
    let l = li(lang);
    let split = split_for(&world.id, config.dev_share);
    let provenance = Provenance::OwnedProcedural {
        generator: GENERATOR.into(),
        seed: config.seed,
    };
    let mut state = SocialState::new(&world.owner, &world.owner)?;
    let gold = &world.gold;
    let oracle = |_: &str, chunk: &str| -> Result<(f32, Option<String>)> {
        let mut lines = Vec::new();
        for line in chunk.lines() {
            if let Some(turn) = line
                .strip_prefix('[')
                .and_then(|r| r.split_once(']'))
                .and_then(|(n, _)| n.parse::<u64>().ok())
            {
                if let Some(g) = gold.get(&turn) {
                    lines.extend(g.iter().cloned());
                }
            }
        }
        Ok(if lines.is_empty() {
            (0.0, None)
        } else {
            (1.0, Some(lines.join("\n")))
        })
    };
    let reader = ReaderConfig {
        lang,
        ..config.reader
    };
    let mut chunk_index = 0;
    let mut update_examples = Vec::new();
    read_recording(&mut state, &world.turns, &oracle, &reader, &mut |record| {
        update_examples.push(Example {
            id: format!("{}:update:{chunk_index}", world.id),
            task: Task::Update,
            lang,
            split,
            adapter: None,
            question: None,
            options: vec![],
            state: Some(record.state),
            chunk: Some(record.chunk),
            draft: None,
            target: Target {
                change: Some(record.changed),
                text: Some(record.delta.unwrap_or_else(|| "none".into())),
                ..Default::default()
            },
            provenance: provenance.clone(),
            weight: 1.0,
        });
        chunk_index += 1;
    })?;
    out.examples.extend(update_examples);

    let mut rng = StdRng::seed_from_u64(
        crate::sha256_hex(world.id.as_bytes())
            .bytes()
            .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(u64::from(b))),
    );
    let yes_no = vec![lang.yes().to_string(), lang.no().to_string()];
    let decide = |id: String,
                  question: String,
                  focus: &str,
                  options: Vec<String>,
                  choice: Choice,
                  out: &mut SynthOutput| {
        let state_text = state.render(&[focus], config.reader.state_bytes, lang);
        let example = Example {
            id: id.clone(),
            task: Task::Decide,
            lang,
            split,
            adapter: None,
            question: Some(question.clone()),
            options: options.clone(),
            state: Some(state_text.clone()),
            chunk: None,
            draft: None,
            target: Target {
                choice: Some(choice),
                ..Default::default()
            },
            provenance: provenance.clone(),
            weight: 1.0,
        };
        out.cases.push(Case {
            id: id.replace(":decide:", ":case:"),
            task: Task::Decide,
            lang,
            split,
            adapter: None,
            question: Some(question.clone()),
            options,
            state: Some(state_text),
            chunk: None,
            gold: Some(example.target.clone()),
            provenance: provenance.clone(),
        });
        out.examples.push(example);
        out.examples.push(gate_example(
            &format!("{id}:gate"),
            lang,
            split,
            &question,
            true,
            true,
            &provenance,
        ));
    };

    let people: Vec<String> = world.traits.keys().cloned().collect();
    // Knowledge, balanced between yes and no per topic.
    for &t in &world.topics {
        let spec = &TOPICS[t];
        let topic = spec.topic[l];
        let Some(current) = state
            .facts
            .get(topic)
            .and_then(|f| f.current())
            .map(|v| v.value.clone())
        else {
            continue;
        };
        let said = spec.verb[l].replace("{v}", &current);
        let (mut knows, mut not): (Vec<&String>, Vec<&String>) = people
            .iter()
            .partition(|p| state.knows(p, topic) == Some(true));
        knows.shuffle(&mut rng);
        not.shuffle(&mut rng);
        let n = knows.len().min(not.len()).min(2);
        for p in knows[..n].iter().chain(not[..n].iter()) {
            let question = match lang {
                Lang::Es => format!("¿Sabe {p} que {topic} {said}?"),
                Lang::En => format!("Does {p} know that {topic} {said}?"),
            };
            let choice = if state.knows(p, topic) == Some(true) {
                0
            } else {
                1
            };
            decide(
                format!("{}:decide:know:{t}:{p}", world.id),
                question,
                p,
                yes_no.clone(),
                Choice::Option(choice),
                out,
            );
        }
        // Belief: what each believes, or "cannot know" for someone who never saw it.
        for p in people.iter().take(3) {
            let mut options: Vec<String> = spec.values[l].iter().map(|v| v.to_string()).collect();
            options.shuffle(&mut rng);
            options.truncate(4);
            let belief = state.belief(p, topic).map(str::to_string);
            if let Some(b) = &belief {
                if !options.contains(b) {
                    options[0] = b.clone();
                    options.shuffle(&mut rng);
                }
            }
            let choice = match &belief {
                Some(b) => Choice::Option(options.iter().position(|o| o == b).expect("present")),
                None => Choice::Abstain,
            };
            let question = match lang {
                Lang::Es => format!("¿Qué cree {p} sobre {topic}?"),
                Lang::En => format!("What does {p} believe about {topic}?"),
            };
            decide(
                format!("{}:decide:belief:{t}:{p}", world.id),
                question,
                p,
                options,
                choice,
                out,
            );
        }
    }
    // What to expect: from the hidden trait, when it showed at least twice.
    for (p, traits) in &world.traits {
        for &(trait_index, has) in traits {
            let spec = &TRAITS[trait_index];
            let (yes, no) = world
                .shown
                .get(&(p.clone(), trait_index))
                .copied()
                .unwrap_or_default();
            let choice = if yes + no >= 2 {
                Choice::Option(if has { 0 } else { 1 })
            } else {
                Choice::Abstain
            };
            let question = spec.question[l].replace("{X}", p);
            decide(
                format!("{}:decide:expect:{}:{p}", world.id, spec.key),
                question,
                p,
                yes_no.clone(),
                choice,
                out,
            );
        }
    }
    // Composed answers, grounded in the exact state.
    for p in people.iter().take(2) {
        for (kind, question, answer) in composed(&state, world, p, lang) {
            let id = format!("{}:compose:{kind}:{p}", world.id);
            let state_text = state.render(&[p.as_str()], config.reader.state_bytes, lang);
            out.examples.push(Example {
                id: id.clone(),
                task: Task::Compose,
                lang,
                split,
                adapter: None,
                question: Some(question.clone()),
                options: vec![],
                state: Some(state_text.clone()),
                chunk: None,
                draft: None,
                target: Target {
                    plan: Some(answer.0.clone()),
                    text: Some(answer.1.clone()),
                    ..Default::default()
                },
                provenance: provenance.clone(),
                weight: 0.5,
            });
            out.cases.push(Case {
                id: format!("{}:case:{kind}:{p}", world.id),
                task: Task::Compose,
                lang,
                split,
                adapter: None,
                question: Some(question.clone()),
                options: vec![],
                state: Some(state_text),
                chunk: None,
                gold: Some(Target {
                    plan: Some(answer.0),
                    text: Some(answer.1),
                    ..Default::default()
                }),
                provenance: provenance.clone(),
            });
            out.examples.push(gate_example(
                &format!("{id}:gate"),
                lang,
                split,
                &question,
                true,
                false,
                &provenance,
            ));
        }
    }
    // Questions that are not for Wanda.
    let (question, yes_no_kind) = *NOT_SOCIAL[l].choose(&mut rng).expect("question");
    out.examples.push(gate_example(
        &format!("{}:gate:not-social", world.id),
        lang,
        split,
        question,
        false,
        yes_no_kind,
        &provenance,
    ));
    Ok(state)
}

fn gate_example(
    id: &str,
    lang: Lang,
    split: Split,
    question: &str,
    social: bool,
    yes_no: bool,
    provenance: &Provenance,
) -> Example {
    Example {
        id: id.to_string(),
        task: Task::Gate,
        lang,
        split,
        adapter: None,
        question: Some(question.to_string()),
        options: vec![],
        state: None,
        chunk: None,
        draft: None,
        target: Target {
            gate: Some(social),
            yes_no: Some(yes_no),
            ..Default::default()
        },
        provenance: provenance.clone(),
        weight: 1.0,
    }
}

/// Template answers from the exact state: (kind, question, (plan, answer)).
fn composed(
    state: &SocialState,
    world: &World,
    person: &str,
    lang: Lang,
) -> Vec<(&'static str, String, (String, String))> {
    let l = li(lang);
    let mut out = Vec::new();
    if let Some(&t) = world.topics.first() {
        let spec = &TOPICS[t];
        let topic = spec.topic[l];
        if let Some(current) = state.facts.get(topic).and_then(|f| f.current()) {
            let now = spec.verb[l].replace("{v}", &current.value);
            let belief = state
                .belief(person, topic)
                .map(|b| spec.verb[l].replace("{v}", b));
            let manner = manner_advice(state, person, lang);
            let question = match lang {
                Lang::Es => format!("¿Tengo que avisar a {person} de cómo quedó {topic}?"),
                Lang::En => format!("Do I need to tell {person} how {topic} ended up?"),
            };
            let (plan, answer) = match (state.knows(person, topic), belief, lang) {
                (Some(true), _, Lang::Es) => (
                    format!("tranquilo | {person} ya lo sabe | no hace falta"),
                    format!("No hace falta: {person} estaba cuando se dijo que {topic} {now}."),
                ),
                (Some(true), _, Lang::En) => (
                    format!("calm | {person} already knows | no need"),
                    format!("No need: {person} was there when it was said that {topic} {now}."),
                ),
                (_, Some(old), Lang::Es) => (
                    format!("directo | {person} sigue creyendo que {topic} {old} | avisar"),
                    format!("Sí. {person} sigue creyendo que {topic} {old}: no estaba cuando cambió. Escríbele que ahora {topic} {now}{manner}."),
                ),
                (_, Some(old), Lang::En) => (
                    format!("direct | {person} still believes {topic} {old} | tell them"),
                    format!("Yes. {person} still believes {topic} {old}: they missed the change. Tell them {topic} {now}{manner}."),
                ),
                (_, None, Lang::Es) => (
                    format!("directo | {person} no sabe nada de {topic} | avisar"),
                    format!("Sí. {person} no vio ningún mensaje sobre {topic}. Cuéntale que {topic} {now}{manner}."),
                ),
                (_, None, Lang::En) => (
                    format!("direct | {person} knows nothing about {topic} | tell them"),
                    format!("Yes. {person} never saw a message about {topic}. Tell them {topic} {now}{manner}."),
                ),
            };
            out.push(("tell", question, (plan, answer)));
        }
    }
    let expectations: Vec<String> = state
        .find(person)
        .and_then(|id| state.people.get(id))
        .map(|p| {
            p.what_to_expect()
                .into_iter()
                .filter(|e| e.weight() >= 2.0)
                .take(3)
                .map(|e| hedge(e.confidence(), &e.statement, lang))
                .collect()
        })
        .unwrap_or_default();
    let question = match lang {
        Lang::Es => format!("¿Qué debería tener en cuenta antes de pedirle algo a {person}?"),
        Lang::En => format!("What should I keep in mind before asking {person} for something?"),
    };
    let (plan, answer) = if expectations.is_empty() {
        match lang {
            Lang::Es => (
                "prudente | poca evidencia | preguntar".to_string(),
                format!("Todavía no hay suficiente en las conversaciones para saber qué esperar de {person}. Pídeselo con claridad y fíjate en cómo responde."),
            ),
            Lang::En => (
                "careful | little evidence | ask".to_string(),
                format!("There isn't enough in the conversations yet to know what to expect from {person}. Ask clearly and see how they respond."),
            ),
        }
    } else {
        match lang {
            Lang::Es => (
                format!("hipótesis | {} | adaptar el pedido", expectations[0]),
                format!("Por lo visto en el grupo, {person}: {}. Son hipótesis, no certezas: tenlo en cuenta al pedírselo.", expectations.join("; ")),
            ),
            Lang::En => (
                format!("hypothesis | {} | adapt the ask", expectations[0]),
                format!("From the group, {person}: {}. These are hypotheses, not certainties: keep them in mind when you ask.", expectations.join("; ")),
            ),
        }
    };
    out.push(("expect", question, (plan, answer)));
    out
}

fn hedge(confidence: f32, statement: &str, lang: Lang) -> String {
    let words = match lang {
        Lang::Es => ["probable", "a veces", "poco probable"],
        Lang::En => ["likely", "sometimes", "unlikely"],
    };
    let word = if confidence >= 0.7 {
        words[0]
    } else if confidence >= 0.4 {
        words[1]
    } else {
        words[2]
    };
    format!("{statement} ({word})")
}

fn manner_advice(state: &SocialState, person: &str, lang: Lang) -> String {
    let Some(p) = state.find(person).and_then(|id| state.people.get(id)) else {
        return String::new();
    };
    let strong = |key: &str| {
        p.expectations
            .iter()
            .find(|e| e.key == key)
            .is_some_and(|e| e.confidence() >= 0.6 && e.weight() >= 2.0)
    };
    match lang {
        Lang::Es if strong("avoids-conflict") => ", con tono tranquilo".into(),
        Lang::Es if strong("slow-replies") => ", con margen: suele tardar en responder".into(),
        Lang::Es if strong("direct") => ", sin rodeos".into(),
        Lang::En if strong("avoids-conflict") => ", in a calm tone".into(),
        Lang::En if strong("slow-replies") => ", early: they tend to reply late".into(),
        Lang::En if strong("direct") => ", plainly".into(),
        _ => String::new(),
    }
}

/// Converts a generated choice target back to a label, for reports.
pub fn choice_label(options: &[String], choice: Choice) -> String {
    match choice {
        Choice::Option(i) => options.get(i).cloned().unwrap_or_default(),
        Choice::Abstain => "<cannot know>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> SynthConfig {
        SynthConfig {
            worlds: 4,
            min_turns: 50,
            max_turns: 90,
            ..Default::default()
        }
    }

    #[test]
    fn deterministic_and_valid() {
        let a = generate(&small()).unwrap();
        let b = generate(&small()).unwrap();
        assert_eq!(a.examples, b.examples);
        assert!(!a.examples.is_empty() && !a.cases.is_empty());
        for e in &a.examples {
            e.validate().unwrap();
        }
        for c in &a.cases {
            c.validate().unwrap();
        }
        for task in [Task::Gate, Task::Update, Task::Decide, Task::Compose] {
            assert!(a.examples.iter().any(|e| e.task == task), "{task:?}");
        }
        let ids: std::collections::HashSet<&str> =
            a.examples.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), a.examples.len(), "example ids are unique");
    }

    #[test]
    fn knowledge_labels_are_balanced_and_gold_deltas_all_apply() {
        let out = generate(&SynthConfig {
            worlds: 12,
            ..small()
        })
        .unwrap();
        let know: Vec<&Example> = out
            .examples
            .iter()
            .filter(|e| e.task == Task::Decide && e.id.contains(":decide:know:"))
            .collect();
        assert!(know.len() > 20);
        let yes = know
            .iter()
            .filter(|e| e.target.choice == Some(Choice::Option(0)))
            .count();
        assert_eq!(yes * 2, know.len(), "yes and no are balanced");
        // The oracle's deltas are all valid: none were rejected while reading.
        let config = small();
        let world = build_world(config.seed, 3, Lang::Es, &config);
        let mut state = SocialState::new(&world.owner, &world.owner).unwrap();
        let gold = world.gold.clone();
        let oracle = |_: &str, chunk: &str| -> Result<(f32, Option<String>)> {
            let lines: Vec<String> = chunk
                .lines()
                .filter_map(|l| l.strip_prefix('[')?.split_once(']')?.0.parse::<u64>().ok())
                .filter_map(|t| gold.get(&t))
                .flatten()
                .cloned()
                .collect();
            Ok((1.0, (!lines.is_empty()).then(|| lines.join("\n"))))
        };
        let report =
            crate::reader::read(&mut state, &world.turns, &oracle, &config.reader).unwrap();
        assert_eq!(report.rejected, 0, "{report:?}");
        assert!(report.applied > 5);
    }
}
