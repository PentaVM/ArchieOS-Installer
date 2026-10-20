# Training Wanda's LNCM

Everything is one binary, `archie-wanda-lncm`, built from `core/`:

```bash
cd core
cargo build --release -p archie-wanda-lncm                    # CPU
cargo build --release -p archie-wanda-lncm --features cuda    # NVIDIA GPU (needs the CUDA toolkit)
B=target/release/archie-wanda-lncm
```

Design and invariants: [`docs/WANDA-LNCM.md`](../../docs/WANDA-LNCM.md).

## 0. One command on the owner's PC (Windows + WSL, RTX 3070)

```powershell
powershell -ExecutionPolicy Bypass -File training\wanda_lncm\train-local.ps1 -BudgetUsd 22
```

It reads the key from the `*nebius*api*.txt` file on the desktop (process environment only:
never printed, written or committed), checks it against Token Factory, builds with CUDA when
`nvcc` is present, generates the data, imports the `tomgen` tranches it finds, distils under a
**hard dollar limit** (70 % teachers, 5 % drafts, the rest judges; a call starts only if its
worst case still fits, and the ledger makes the limit hold across reruns), pauses the Bonsai2
worker through its `hold` file, and trains with a thermal pause at 83 °C (resume at 72 °C).
`touch <checkpoint>/STOP` stops cleanly; rerunning the same command resumes every stage.
In WSL directly: `training/wanda_lncm/train-local.sh` (same knobs as environment variables).

At the registry's prices, ~$22 buys roughly 600–900 composed cases with four teacher answers,
drafts and judging; decisions and state updates come from exact simulator labels for free. The
ledger total uses registry prices: Nebius's invoice is authoritative.

### What "frontier level in this area" means here

The target is to match the teachers on this narrow task at a fraction of their latency, not to
be a general model. It is measured, not assumed:
- `eval` reports decision accuracy and calibration on exact labels, and char-F1 of state 2
  against the selected teacher answers on held-out cases (how close the student is to the
  teachers' best).
- FANToM and OpenToM stay evaluation-only and are run with the existing harness in
  `training/wanda_bonsai2`, naming the checkpoint and protocol.
A result is only a claim when both come from a named checkpoint on a named device.

## 1. Data

Owned synthetic worlds (Spanish and English, exact labels for every task):

```bash
$B synth --out data/synth --worlds 5000 --seed 20261020
# data/synth/examples.jsonl  gate · update · decide · compose examples
# data/synth/cases.jsonl     the same questions, for teachers
```

Existing Python `chat-v1` tranches (`training/wanda_bonsai2`, `tomgen`) become cases:

```bash
$B import-tranche --tranche /path/tom-gen-005-behav.json --out data/tomgen-cases.jsonl
```

Public datasets enter as `Case` records with `provenance = {kind = "public_licensed", dataset,
revision, license}`; write a converter per dataset after its licence review. Private chats are
never cases.

## 2. Distillation (Nebius Token Factory)

Copy [`distill.example.toml`](distill.example.toml), point `models_dir` at the registry
(`models/`), export `NEBIUS_TOKEN_FACTORY_KEY`, list the benchmark files under `eval_only`.

```bash
$B distill teach  --config distill.toml --cases data/synth/cases.jsonl --out data/teachers.jsonl
$B distill draft  --config distill.toml --cases data/synth/cases.jsonl --out data/drafts.jsonl
$B distill judge  --config distill.toml --cases data/synth/cases.jsonl --samples data/teachers.jsonl --out data/judges.jsonl
$B distill select --config distill.toml --cases data/synth/cases.jsonl \
   --samples data/teachers.jsonl --samples data/drafts.jsonl --samples data/judges.jsonl \
   --out data/distilled.jsonl
```

Every step appends as it goes and skips what is done on a rerun. `max_completion_tokens` caps
spend in tokens and `max_cost_usd` (with `ledger`) caps it in dollars across reruns. Cost ≈
cases × samples × tokens per answer: start with a few thousand cases and measure.

Selection: an exact label wins when the simulator knows it; otherwise decisions need a
majority, delta lines must recur across samples, and composed answers need the best mean
rubric from judges of another model family with no low safety score. Draft answers become the
`draft` input of extra examples, so the student learns draft → expert.

## 3. Train

Presets (approximate parameters with an 8k BPE codec): `small` ~22 M (fits an 8 GB GPU),
`base` ~130 M, `large` ~390 M (data-centre GPU).

```bash
D=ckpt/wanda-lncm-small
$B init --dir $D --preset small --vocab 8192 --examples data/synth/examples.jsonl --examples data/distilled.jsonl
$B train tokenizer --dir $D --examples data/synth/examples.jsonl --examples data/distilled.jsonl \
   --device cuda:0 --steps 20000 --batch 32 --lr 3e-4
$B train generator --dir $D --examples data/synth/examples.jsonl --examples data/distilled.jsonl \
   --device cuda:0 --steps 60000 --batch 16 --lr 3e-4 --mix gate=0.1,update=0.3,decide=0.3,compose=0.3
$B train adapter --dir $D --name personal-owner --examples data/personal-owner.jsonl --device cuda:0 --steps 5000
$B calibrate --dir $D --examples data/synth/examples.jsonl --device cuda:0
$B eval --dir $D --examples data/synth/examples.jsonl --device cuda:0
```

- Stages resume from their last save (`--no-resume` starts over). Metrics go to
  `$D/train-log.jsonl`; final metrics, data digests and file digests go to `$D/manifest.json`.
- Adapter examples are those whose `adapter` field names the adapter; keep personal and work
  data in separate adapters.
- On the RTX 3070 keep the thermal guard of `training/wanda_bonsai2/ops` in mind; `small` with
  `--batch 8` is the realistic size there. `base` and `large` belong on Nebius AI Cloud.

## 4. Use

```bash
$B ask  --dir $D --question "¿Sabe Pedro que el viaje es en julio?" --state state.json
$B read --dir $D --turns chat-turns.jsonl --state state.json --owner owner
$B inspect --dir $D
```

In the Wanda service, add the `[lncm]` section shown in `docs/WANDA-LNCM.md`. Wanda refuses to
start if any checkpoint file does not match its manifest.

## What a number means

Only `eval` output on development data, for a named checkpoint, on a named device, is a
measurement. Training loss is not accuracy, and the synthetic development set is not a
benchmark. FANToM and OpenToM results come from the existing harness in
`training/wanda_bonsai2` and must name the checkpoint and protocol used.
