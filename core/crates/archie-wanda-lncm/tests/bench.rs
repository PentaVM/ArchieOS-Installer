//! Latency and host-memory traffic of the runtime entry points, at a real checkpoint's shape.
//!
//! The weights are random (the cost of a forward pass does not depend on them); the codec and
//! config come from a checkpoint directory, so prompts have their real lengths:
//!
//! ```sh
//! LNCM_BENCH_DIR=~/wanda-lncm/ckpt-small cargo test --release -p archie-wanda-lncm \
//!     --test bench -- --ignored --nocapture
//! ```
//!
//! `LNCM_BENCH_DEVICE=cuda:0` measures on the GPU when built with `--features cuda`.

use archie_wanda_lncm::codec::Codec;
use archie_wanda_lncm::config::LncmConfig;
use archie_wanda_lncm::generator::Generator;
use archie_wanda_lncm::model::{ComposeOptions, Lncm};
use archie_wanda_lncm::vq::VqVae;
use archie_wanda_lncm::Lang;
use candle_core::{DType, Tensor};
use candle_nn::{VarBuilder, VarMap};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new, Ordering::Relaxed);
        System.realloc(ptr, layout, new)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn measure<T>(name: &str, runs: usize, mut f: impl FnMut() -> T) {
    let _ = f(); // warm-up: kernels compiled, buffers first touched
    let mut times = Vec::with_capacity(runs);
    let calls = CALLS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);
    for _ in 0..runs {
        let started = Instant::now();
        let _ = f();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let calls = (CALLS.load(Ordering::Relaxed) - calls) / runs;
    let mib = (BYTES.load(Ordering::Relaxed) - bytes) as f64 / runs as f64 / (1024.0 * 1024.0);
    println!(
        "{name:<28} p50 {:>8.2} ms  p90 {:>8.2} ms  host allocs {calls:>7}  host MiB {mib:>8.2}",
        times[runs / 2],
        times[(runs * 9 / 10).min(runs - 1)]
    );
}

#[test]
#[ignore = "benchmark: needs LNCM_BENCH_DIR with a checkpoint's codec.json and config.toml"]
fn runtime_entry_points() {
    let dir = std::path::PathBuf::from(std::env::var("LNCM_BENCH_DIR").expect("LNCM_BENCH_DIR"));
    let device = archie_wanda_lncm::train::device(
        &std::env::var("LNCM_BENCH_DEVICE").unwrap_or_else(|_| "cpu".into()),
    )
    .unwrap();
    let config =
        LncmConfig::from_toml(&std::fs::read_to_string(dir.join("config.toml")).unwrap()).unwrap();
    let codec = Codec::load(&dir.join("codec.json")).unwrap();
    // Random weights, then held as plain tensors, as a loaded checkpoint holds them (no
    // gradient tracking), so the runtime takes the same paths it takes in service.
    // The same pseudo-random weights in every run (seeded by each tensor's name), so every
    // run takes the same branches; norms start at one as they do in training.
    let map = VarMap::new();
    let vb = VarBuilder::from_varmap(&map, DType::F32, &device);
    VqVae::new(&vb, &config).unwrap();
    Generator::new(&vb, &config).unwrap();
    let tensors = map
        .data()
        .lock()
        .unwrap()
        .iter()
        .map(|(name, var)| {
            let shape = var.as_tensor().shape().clone();
            let count = shape.elem_count();
            let values: Vec<f32> = if name.contains("norm") {
                vec![1.0; count]
            } else {
                let seed = name.bytes().fold(1469598103934665603u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(1099511628211)
                });
                let mut rng = StdRng::seed_from_u64(seed);
                (0..count).map(|_| rng.gen_range(-0.04f32..0.04)).collect()
            };
            (name.clone(), Tensor::from_vec(values, shape, &device).unwrap())
        })
        .collect();
    let vb = VarBuilder::from_tensors(tensors, DType::F32, &device);
    let vq = VqVae::new(&vb, &config).unwrap();
    let generator = Generator::new(&vb, &config).unwrap();
    let model = Lncm::from_parts(config.clone(), codec, vq, generator, device.clone());
    let runs: usize = std::env::var("LNCM_BENCH_RUNS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(10);

    let state = (0..14)
        .map(|i| format!("- Persona{i}: vio que la cena = jueves; sabe que el presupuesto = {} pesos.", 1000 + i))
        .collect::<Vec<_>>()
        .join("\n");
    let state = format!("Personas: Ana, Pedro, Luis\nLo que cada persona ha visto:\n{state}");
    let draft = "Sí, conviene avisarle: no estaba cuando se cambió la fecha de la cena.";
    let question = "¿Tengo que avisar a Ana de que la cena pasó al jueves?";
    let chunk = (0..20)
        .map(|t| format!("[{t}] Pedro: la cena pasa al jueves y el presupuesto sube a {} pesos", 1000 + t))
        .collect::<Vec<_>>()
        .join("\n");
    let roster = vec!["Ana".to_string(), "Pedro".to_string(), "Luis".to_string()];

    println!(
        "device {device:?}, prompt_len {}, target_len {}, levels {:?}, d_model {}, layers {}",
        config.prompt_len, config.target_len, config.levels, config.generator.d_model, config.generator.layers
    );
    measure("gate", runs, || model.gate(Lang::Es, question).unwrap());
    measure("quick (gate + decide)", runs, || {
        model.quick(Lang::Es, question, Some(&state), Some(draft)).unwrap()
    });
    measure("compose 1 candidate", runs, || {
        model
            .compose(Lang::Es, question, Some(&state), Some(draft), &roster, ComposeOptions::default())
            .unwrap()
    });
    measure("compose 4 candidates", runs.div_ceil(2), || {
        model
            .compose(
                Lang::Es,
                question,
                Some(&state),
                Some(draft),
                &roster,
                ComposeOptions {
                    candidates: 4,
                    ..Default::default()
                },
            )
            .unwrap()
    });
    measure("update (change + delta)", runs, || {
        model.update(Lang::Es, &state, &chunk, 0.0).unwrap()
    });
    measure("service answer (all)", runs.div_ceil(2), || {
        let gate = model.gate(Lang::Es, question).unwrap();
        let quick = model.quick(Lang::Es, question, Some(&state), Some(draft)).unwrap();
        let composed = model
            .compose(Lang::Es, question, Some(&state), Some(draft), &roster, ComposeOptions::default())
            .unwrap();
        (gate, quick, composed)
    });
}
