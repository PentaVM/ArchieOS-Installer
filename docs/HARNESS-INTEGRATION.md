# Harness integration

Archie has two harness paths. Google Harness² runs on Archie's own unified inference
service. The older HarnessRouter/UHP adapter is a separate project, described at the end.

## Google Harness² on the unified inference service

`tools/harness2/` runs the actual upstream engine, `google-research/harness2` pinned to
`1cfe254e8d25b15f37048c3730134686abe60335`, through Archie adapters. Every stage goes
through the same unified path as any other inference: operator profile and domain, Policy
lease checked right before dispatch, the model the router admits, a receipt in PostgreSQL
and the result sealed in the Vault. The worker owns no credentials, endpoints or
authority. A run is `k=1`: four solver rollouts and three improvement sessions plus one
reflection, eight calls, and upstream's ledger reports `R=4/P=3`. Promotion of a learned
harness or lesson is denied by default, and every result says
`improvement_verified: false`. The worker's protocol and reply contract are in
[tools/harness2/README.md](../tools/harness2/README.md).

### Local models

Bonsai is Archie's only LLM. Ternary Bonsai 2 9B runs on llama.cpp (`llama-server` on
`127.0.0.1:8081`, alias `bonsai`), and two manifests describe it:

| Manifest | Reasoning | Capability | Use |
|---|---|---|---|
| `models/local/bonsai-2-9b.toml` | on | `cognition.reasoning` | decisions, Harness² |
| `models/local/bonsai-2-9b-fast.toml` | off | `cognition.response` | short answers, voice |

Both are `Local`, take no credential and accept up to `Restricted` context. `Secret` reaches
no model. Both ship with `availability = "offline"`, because the image does not include
llama-server and the node's own models are preferred for every private and voice request.
On a node whose llama-server serves these weights, set `availability = "ready"` in the
installed copies under `/etc/archie/models/local/` (or `<root>/etc/models/local/`). Until
then the router never chooses Bonsai. The manifest's `thinking = "on" | "off"` label is sent to llama.cpp as
`chat_template_kwargs.enable_thinking`, on both the plain and the streamed path. A
misspelt value is refused when the endpoint is built. With reasoning on, llama.cpp may
spend half of each request's output budget thinking. The thought is then closed with
Archie's message, and the other half stays for the answer. A pinned model is asked in the
role its manifest offers, so `--model local.bonsai-2-9b` reaches the reasoning manifest.
An unpinned request never drifts onto it.

### Who uses the single path

- `archie ai ask|recall|harness2` and `archie-core`'s own AI control.
- Wanda, when its configuration has a `[core]` block: each query is an `AiInfer` pinned
  to the model Cerebra admitted, graded Restricted, with a receipt (`docs/WANDA-CONTEXT.md`).
- Not the voice's own turns. A voice turn streams and calls tools; `AiInfer` returns one
  sealed answer and offers no tools, so the voice still asks the router directly
  (`archie-voice/src/nemotron.rs`). What the voice may do stays under Policy and Stage.
  Its local model is now a llama.cpp descriptor (`--endpoint`, `--thinking off|on`), so
  the liveness probe applies to it, and its `wanda_context` tool reaches Wanda, which goes
  through Core.

### Operator commands

On a node, `archied` serves these requests (`AiInfer`, `AiRecall`, `AiHarness`). Without a
daemon, `--standalone` runs the same path for diagnostics. `--root DIR` relocates the
whole tree, and `--json` prints machine-readable results.

```bash
archie ai init --domain personal/owner
archie ai configure-harness2 --python /opt/archie/python3.13/bin/python3.13 --worker /usr/lib/archie/harness2/worker.py
archie --json ai ask --domain personal/owner --privacy restricted --runner api --model local.bonsai-2-9b-fast --max-tokens 256 --prompt-file question.txt > answer.json
archie ai recall --receipt answer.json
archie --json ai harness2 --domain personal/owner --task-family mail-next-action --run-id mail-2026-10-22a --runner api --model local.bonsai-2-9b --duration-ms 300000 --prompt-file task.txt
```

- `ai init` creates the operator profile `etc/inference.toml` once and never overwrites it.
  Its defaults are 2,048 output tokens and 60 s per call. With reasoning on, Harness² on
  Bonsai needs `max_output_tokens = 4096` and `max_duration_ms = 120000`. Edit the
  profile (mode 0600) to set them.
- `etc/database.json` names the PostgreSQL connection and schema.
- `configure-harness2` records the interpreter and worker once; the paths above are
  examples, and a second call is refused. The interpreter is a POSIX
  CPython 3.12 or 3.13; upstream refuses 3.14. The worker directory holds exactly
  `worker.py`, `upstream-manifest.json` and the 17 listed `vendor/` files. Everything is
  checked against the compiled pin before launch. The directory and every ancestor must
  belong to the operator or root and be writable by no one else.
- A Harness² task is public text of at most 16 KiB. Every stage is asked and sealed as
  `Public`, so a later request may cite its results anywhere, a cloud runner included:
  never put private text in a task. Its worst stage is twice the task plus about 16 KiB,
  which must fit `max_prompt_bytes`.
- Repeating a completed run's exact identity resumes it without calling a model. Changed
  input under the same run id is refused.

### Emergency pause

```bash
archie ai pause     # refuse every AI dispatch now
archie ai status    # paused? recorded?
archie ai resume
```

- These go to `archied` only (`AiPause`, `AiResume`, `AiStatus`). Only the kernel-identified
  owner of `etc/inference.toml` may send them, as with the other AI requests. A paused or
  disabled profile still belongs to its owner, so the owner can always resume.
- The pause is Core's emergency pause on the one Policy engine the daemon shares with every
  AI dispatch. It stops a request admitted before it at dispatch, and a model answer that
  arrives during it is never committed or shown. As Core policy, it also refuses every other
  mutating lease, such as a Sentry actuation, while it holds.
- It is recorded in `/var/lib/archie/policy/emergency-pause` (`var/lib/policy/` under
  `--root`) and survives a restart. `--standalone`
  diagnostics honour the record. A resume removes the record and writes its audit entry
  before it takes effect. If either fails, the node stays paused.

### A local model whose server is down

A Bonsai manifest marked `ready` says nothing about whether llama-server is up. Before
routing, the runtime asks each usable local llama.cpp model's server for `/health` (500 ms),
and keeps the answer for 5 s. A refused connection, a timeout or a 5xx answer (llama-server
says 503 while it loads) makes that model offline to routing until it answers again. Voice,
`archie-ask` and `AiInfer` then use the next admissible candidate. That is a veiled cloud
model when the grade allows it (`sensitive` or below), and a refusal otherwise. A run
pinned to the dead model is refused, never moved to another model. Cloud models are never
probed.

### Measured on the RTX 3070 (2026-10-22)

One complete run on `local.bonsai-2-9b` with reasoning on, through `archie ai harness2
--standalone` against a disposable PostgreSQL 18 cluster. The task was a synthetic public
email: choose `ask_owner`, `draft` or `nothing` for a trade-fair invitation the owner had
not answered.

| Stage | Input tokens | Output tokens | Time |
|---|---:|---:|---:|
| 1 base solver | 783 | 2,130 | 18.7 s |
| 2 proposer | 4,245 | 2,280 | 23.2 s |
| 3 proposer | 4,107 | 2,267 | 23.3 s |
| 4 solver | 783 | 2,131 | 21.1 s |
| 5 solver | 783 | 2,121 | 21.0 s |
| 6 proposer (compose) | 4,200 | 947 | 11.4 s |
| 7 solver | 783 | 2,142 | 20.9 s |
| 8 reflection | 3,707 | 875 | 10.4 s |

- The run took 150.7 s end to end, 150.1 s of it in model calls. Upstream's ledger
  reported `R=4`, `P=3` and 8 calls. All eight receipts were completed and sealed.
- Every solver rollout chose `ask_owner`. All three proposals returned the seed harness
  unchanged. The reflection recorded "no observed effect" and still wrote a lesson. The
  default promotion gate refused it (`baseline_retained`, `no_trusted_assessor`), and no
  library entry was written.
- Rerunning the same identity resumed in 73 ms without a model call.
- A second run on the final branch code also completed: 8 stages, `R=4`, `P=3`,
  169.3 s, promotion refused by default. With the manifests as shipped (`offline`), the
  same request was refused in 117 ms, before any model call.
- The run says nothing about model quality or improvement, which nobody assessed.

Twelve earlier runs that night stopped on reply shapes the worker refused. They exposed
the defects fixed in the branch: the plain-text seed, unbounded reasoning, the pinned role,
the key prefix, list and object fields, a fenced reply, a guardrails newline, an unreadable
proposal and `NO_ENTRY`. Bonsai's output still varies between runs.

### Schema-constrained JSON stages

The solver's `artifact` reply and the proposer's `harness` reply are JSON, so an API
runner now decodes them under a JSON Schema. `harness2::reply_schema` gives each stage
exactly the keys the worker reads, every one a string; the proposal's map names the four
files of the worker's own `ALLOWED` set, and its `guardrails.md` may only be the base
harness's exact text, because the worker aborts the run on any edit to it. The artifact,
the trajectory and every editable file are at least one character long, because the
worker aborts the run on an empty one; a proposal's notes may be empty. The reflection is
upstream's tagged text and gets no schema.

- The substrate attaches the schema only for API runners, after the bridge has matched
  the stage's format to the pinned schedule. Codex receives none.
- `archie-model-runtime` sends it only on the OpenAI wire, as
  `response_format: {type: "json_schema", json_schema: {name, strict: true, schema}}`,
  next to the manifest's reasoning switch. llama.cpp turns it into a grammar; vLLM and
  the hosted OpenAI-compatible APIs accept the same form, without the string
  `minLength`, which only llama.cpp's converter is known to enforce and a strict
  validator elsewhere may refuse. Anthropic never receives it.
- An operator's request cannot carry a schema: `InferenceRequest.reply_schema` is never
  serialized, and a schema that does not describe an object or is over 16 KiB is
  refused before any connection.
- The worker still checks every reply. The schema keeps a small local model from
  writing a shape the worker would refuse; it proves nothing about the content.

To run the same 8 stages against a real llama.cpp server, use the ignored test. It needs a
disposable PostgreSQL in `ARCHIE_TEST_DATABASE_URL` and the worker's Python:

```bash
ARCHIE_HARNESS2_PYTHON=/path/to/python3.13 ARCHIE_TEST_LLAMA_URL=http://127.0.0.1:8081/v1/ ARCHIE_TEST_LLAMA_MODEL=bonsai ARCHIE_TEST_LLAMA_THINKING=on ARCHIE_TEST_LLAMA_STAGE_MS=120000 cargo test -p archie-workstation --test unified_runtime -- --ignored --exact harness2_completes_on_a_real_llama_cpp_server_under_reply_schemas --nocapture
```

- It prints each stage's receipt.
- `ARCHIE_TEST_LLAMA_MAX_OUTPUT` (default 4096) sets the output budget and the profile's
  ceiling with it.
- The whole run is bounded at 300 s.

In the first attempt on Bonsai (2026-10-23), stages 1 and 2 completed under their schemas.
Then the RTX 3070 was lost to the system ("GPU is lost", a reboot is required), so the run
says nothing yet about all eight stages.

## HarnessRouter (UHP)

Archie owns a small adapter boundary and can use HarnessRouter Community Edition's
Unified Harness Protocol (UHP). The client calls the loopback
`/api/harness/v1/responses` endpoint, supplies `metadata.harness_id`, bounds responses,
disables redirects, supports cancellation, and reports timeout and HTTP failures.
HarnessRouter remains replaceable.

Codex CLI, Claude Code and Hermes are first-run dependencies. ArchieOS does not
redistribute them. `archie harness login codex` delegates to `codex login`; Claude opens
its official interactive CLI login, and Hermes delegates to `hermes setup`. Archie never
reads, copies or converts their OAuth/session tokens.

```bash
archie harness list
archie harness add codex
archie harness login codex
archie harness test codex
```

The process probe uses an argv vector and no shell. Private context may reach a local UHP
worker only after the normal capsule and model/harness routing gates. `Secret` is rejected
before a TCP connection is opened.

HarnessRouter CE is not embedded in the image: its container and proprietary CLI
dependencies add substantial size and require accepting upstream terms. Operators can
install it at first run and point the UHP adapter at its loopback listener.
