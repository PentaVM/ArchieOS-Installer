# IRIS runtime in Rust

`core/crates/archie-iris-runtime` runs IRIS Beta 2.1 on the node without Python. It reads
the released weights file as PyTorch wrote it (`iris-beta21-step132000.pt`, SHA-256
`c6cadb02…3b97`), runs the same model, and serves the same loopback API as `iris_serve`.
`archie-voice live --iris` and `archie-arena-live` talk to it unchanged.

```bash
cargo build --release -p archie-iris-runtime            # CPU
CUDA_COMPUTE_CAP=86 cargo build --release -p archie-iris-runtime --features cuda   # NVIDIA, needs the CUDA toolkit
archie-iris-serve --weights iris-beta21-step132000.pt \
  --sha256 c6cadb02f5a5bffc4ea3c3348d32b475ef7531f643675c06c65451037e4f3b97 \
  --text-tower <dir with SigLIP2 model.safetensors, tokenizer.json, tokenizer_config.json>
```

It binds to `127.0.0.1:8765`. The endpoints are `GET /health`, `POST /detect`, `POST /act`,
`POST /act/reset` and `POST /act/recorded`, with the same parameters as `iris_serve`.

## What is ported

| Part | Source | Rust |
|---|---|---|
| Weights | `torch.save` zip + pickle | read as data by candle's pickle reader, never executed; SHA-256 checked before parsing |
| Letterbox | Pillow `BILINEAR` | the same coefficients, 22-bit fixed point and pass order |
| V-JEPA 2.1 ViT-B/16 | `facebookresearch/vjepa2@204698b` | image patch embedding, 3-D RoPE interpolated onto the pretrained 16 grid, modality embedding |
| SigLIP2 vision | `transformers` `SiglipVisionModel` | bicubic position resize as `interpolate_pos_encoding` |
| SigLIP2 text | `transformers` `SiglipTextModel` + Gemma tokenizer | `tokenizers` crate; attention mask as the tokenizer config says |
| Fusion | 6 layers, 32-expert top-2 MoE | the same capacity per expert over the whole batch, so overflowing tokens drop the same way |
| Decoder | two-stage DETR, 100 queries, 6 layers | same top-k query selection and box refinement |
| Instance embeddings | `roi_align` 3×3 + head | torchvision's `aligned=True` sampling |
| Detection | `detect`, `detect_everything` | same thresholds, NMS, 256-phrase everyday vocabulary, rounding |
| Acting | `ActionMemoryHead` + `IrisActor` | T1–T4 memory per session, slot pooling, pointer, grounding override, 8-control chunk |
| LoRA skill adapters | `LoRABank` + `load_skill` | `--skills-dir`, `/act?adapter=<name\|auto>`, base-fingerprint & shape verification |
| REM recording | `spool_window` + `recorded` | `--rem-spool`, `/act?record=<run>&app=<skill>`, `/act/recorded`, `.npz` windows + JSON sidecars |
| Browser demo page | `iris_serve.PAGE` | `GET /` and `GET /index.html` |

## Parity with the Python IRIS

The integration test `tests/parity.rs` compares the runtime with `archie_iris_foundry` on
CPU, in float32, on the real Beta 2.1 weights, using a 1920×1080 GUI screenshot. It was run
on 2026-10-12:

| Check | Result |
|---|---|
| Letterboxed canvas | 0 of 786,432 bytes differ |
| V-JEPA / SigLIP / fused-ready tokens | relative max error 9e-5 / 7e-5 / 1.1e-4 |
| Fusion, logits, boxes | relative max error 1.7e-6 / 1.4e-6 / 8e-7 |
| `detect`, 6 phrases, threshold 0.05 | 104 of 104 detections identical in phrase and box to the pixel; scores within 1e-4 |
| `detect_everything` | 60 of 60 identical |
| Instance embeddings | worst 1 − cosine 1e-5 |
| `/act`, three frames of one session | identical step, keys, buttons, mouse, scroll, pointer, memory and 7/7 chunk steps |

The SigLIP2 text tower is not in the release, and Hugging Face was not reachable from the
machine that ran the test. So the phrase features were deterministic stand-ins, fed to both
implementations. The detections above are therefore numbers to agree on, not meaningful
detections.

The text tower was checked on its own (`tests/text_tower.rs`) against `transformers` on
random weights and fixed token ids: relative error 1e-6 with and without an attention mask.
`tests/reference/iris_reference.py` regenerates both references.

**The real tower and tokenizer** (`google/siglip2-base-patch16-384` at `f775b65`) were
checked on 2026-10-16 against `transformers` 4.56 on ten phrases, English and Spanish with
accents, an empty one and one with an ellipsis (`tests/text_tower.rs`,
`the_real_tower_and_tokenizer_match_transformers`; `tests/reference/tower_reference.py`
writes the reference):

| Check | Result |
|---|---|
| Token ids, all 64 positions, every phrase | identical |
| Tower in float32 on those ids (PyTorch float32 is 2e-6 from float64) | pooled 2e-7, states 1.5e-6 |
| What fusion receives (`TextTower::encode`) | the same numbers rounded through float16, as IRIS caches phrase features |

The first real detections followed, on CPU, with the real weights and the real tower
(`hf-internal-testing/fixtures_image_utils`, the COCO photograph of two cats on a sofa):
`cat` 0.88 and 0.86, `couch` 0.84, `remote control` 0.66 and 0.63, each on the right
object; in Spanish `gato`, `control remoto` and `sofá` find the same objects, with lower
scores (0.50–0.73; `sofá` 0.33). On a photograph of two dogs on grass, `dog` 0.49 and
0.44 and two patches of `grass`.

`archie-iris-text-pack <repository dir> <out dir>` keeps only the tower's `text_model.*`
tensors (1.13 of 1.5 GB) and the tokenizer files, with a `SOURCE` naming the file they
were cut from; `--text-tower` reads that directory as it reads the whole repository, with
the same results. The WSL Operator image carries it with the weights, so IRIS starts
there with no download (`scripts/wsl/build-operator.sh`).

## Speed

Measured on CPU only, 4 cores with AVX-512, float32, 512×512 canvas:

| | Rust | PyTorch on the same machine |
|---|---:|---:|
| Both encoders | 3.8 s | 1.4 s |
| `detect`, end to end | 5.7 s | — |
| `detect_everything` (17 batch rows) | 31 s | 10.7 s |

Open mode (`mode=all`, what the live voice used on every frame) reads the 256-phrase
everyday vocabulary 16 phrases to a batch row: 17 rows of MoE fusion and decoding for one
frame. A focused read (`mode=phrases`, at most 16 phrases) is one row. Same machine, same
release build, a 1280×720 JPEG:

| | per frame |
|---|---:|
| open (`mode=all`), vocabulary encoding cached | 30.6 s |
| focused, 16 phrases | 5.9 s |

So the live voice now looks openly every `--iris-sweep-ms` (2000 by default; 0 restores an
open look on every frame) and, in between, reads each frame only for the phrases the last
open look found (`Sweep` in `archie-voice/src/live/eyes.rs`). Something new in view is
noticed at the next open look. This works the same against the Python `iris_serve`. The
detector also keeps the last encoded phrase batch, so open mode no longer re-projects the
whole vocabulary on every frame (identical tensors; the text features were already cached).
Scores in a focused read can differ slightly from the same phrase in open mode, because the
phrases sharing a batch row attend to each other.

The CPU path is a fallback. The CUDA build (bf16 on Ampere and later, float32 earlier,
`IRIS_PRECISION` to force) compiles candle's CUDA kernels. `CUDA_COMPUTE_CAP` names the
card when the build machine has none: 86 for an RTX 30xx, 89 for 40xx. Where CUDA 12.0's
`nvcc` refuses the system compiler (GCC 13 and later), `NVCC_CCBIN=/usr/bin/gcc-12` gives
it one it accepts.

**Built, not run on a GPU.** With CUDA 12.0 and `CUDA_COMPUTE_CAP=86` the release build
compiles and links against `libcuda`, cuBLAS, cuBLASLt and cuRAND. On a machine without a
GPU, `--device cuda` stops at start-up with `CUDA_ERROR_NO_DEVICE`, and `archie-session`
picks the CPU build there. No GPU latency is claimed: the first frames per second on the
RTX 3070 are the owner's to measure (`docs/IRIS-BENCHMARKS.md`). Every `/detect` answer
carries its own `latency_ms`, so on the card:

```bash
archie-iris-serve --device cuda --weights … --sha256 … --text-tower … &
for mode in all 'phrases&phrases=person,cup,phone,screen'; do
  for i in 1 2 3 4 5 6; do   # the first answer includes CUDA warm-up
    curl -s --data-binary @frame.jpg "http://127.0.0.1:8765/detect?mode=$mode" | jq .latency_ms
  done
done
```

The live voice processes one inference request at a time and keeps only the newest
waiting frame. `--iris-sweep-ms` starts after an open look finishes, leaving room
for focused looks even when CPU inference takes longer than that interval.
`--iris-timeout-seconds` defaults to60 and accepts1–900; connection establishment
is separately bounded to at most5seconds. This is a request limit, not a measured
throughput or latency guarantee. Both frame endpoints accept at most8MiB.

## Not ported yet

- **The mask, keypoint, reading and temporal heads.** Nothing served reads them.

