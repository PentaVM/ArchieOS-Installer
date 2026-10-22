# Archie adapter for Google Harness²

This directory runs the **actual Google upstream recursion engine**, pinned to
`1cfe254e8d25b15f37048c3730134686abe60335`, with Archie adapters. It is not an
independent Rust reimplementation or a reproduction of the paper's measured gains.
The untouched upstream Python core/config and Apache-2.0 license are under
`vendor/`; `upstream-manifest.json` records Git blob identities and SHA-256 hashes.
The worker verifies those bytes before importing them. No package installation,
benchmark dataset, CLI executor, GPU or model download is needed.

Use a **Linux Python 3.12 or 3.13** interpreter with `/dev/shm` mounted as tmpfs. Upstream uses `fcntl` and declares
Python `<3.14`. Windows callers can launch the worker through their configured WSL
runtime. Provisioning WSL or choosing an installed Python remains an operator task.

```sh
python3.13 tools/harness2/worker.py
python3.13 -m unittest discover -s tools/harness2/tests -v
```

The first command expects a trusted parent on stdin/stdout; it does not start an
interactive model session. The second runs the real pinned engine with a synthetic,
deterministic parent at the external inference boundary. That proves integration
and safety behavior, not model quality, authentication, hardware or latency.

## Execution and editable surface

The worker supplies a custom `BenchmarkAdapter`, data-only substrate and proposer.
It invokes upstream `schedule.run_task`, `recursion.run_sequential`, selection,
reflection and `Library` code. `k=1` produces four solver rollouts (one base, two
proposals, one composition), three improvement sessions and one reflection call:
`R=4`, `P=3`, eight mediated model calls in total. Upstream performs trivial final
selection at `k=1`; there is no additional selector model call.

Upstream's prompt packs recognize only `opencode` and `codex` file surfaces. The
adapter uses the **OpenCode-shaped prompt vocabulary** with these four text files:

- `systemprompt.md`
- `guardrails.md` (immutable, exact bytes from the parent's base harness)
- `LongTermMEMORY.md`
- `tool_descriptions/tool_guidance.md`

The substrate and model channel are `archie_mediated`. No upstream OpenCode, Codex
or Claude executor is invoked. Skills, subagents, scripts, plugins and hooks are
disabled. Proposals return a JSON map of text files; they never edit the host
filesystem or execute generated code. All four files must be present, nonempty and
within the 24,000-byte combined cap. A malformed or off-surface edit aborts the run.

The worker owns no Policy, credentials, provider network endpoint or model weights.
The parent must launch it in OS confinement with a minimal environment, restricted
network and only its trusted state directory and private `/dev/shm` staging writable. This code's data-only
adapters are not a substitute for the parent's process sandbox. Do not pass API
credentials to the worker. No self-upgrade or model training occurs.

## JSONL protocol v1

`protocol.json` documents message shapes. Frames are UTF-8 JSON objects, one per
line, at most 1 MiB (or the smaller requested limit). Duplicate/unknown keys,
nonfinite numbers, malformed UTF-8, wrong request IDs and stale domain/policy
confirmations are refused. Stdout contains protocol frames only; bounded diagnostic
stderr contains exception types rather than private response payloads.

1. Parent sends `start`. It chooses `state_root` from trusted configuration and
   supplies fresh Core admission, canonical ContextDomain, Focus/objective/policy
   references, task/evidence digests, a complete base harness and global limits.
2. Worker sends `inference`. Parent **rechecks active Policy and current domain
   access for every stage**, recomputes the prompt digest, validates reservations,
   selects the admitted model/runtime and uses its ordinary transport. Cloud
   locality remains Cloud even when a child process or proxy is local.
3. Parent sends `inference_result` with matching domain/policy/request ID and
   `admitted:true`. Unknown usage is JSON `null`, never invented zero. An admission
   refusal should return `admitted:false` and matching shape, or terminate/cancel.
4. After upstream adoption/reflection, worker sends `promotion` containing the
   complete proposed harness, experience, baseline/final artifacts and hashes.
   Parent separately admits those exact bytes and checks baseline acceptability.
   Parent replies `promotion_result`. Refusal retains the baseline and commits no
   learned entry. Authorization does not assert measured improvement.
5. Worker emits `result`. Only `completed`/`resumed` are successful. Committed
   library references include manifest paths and SHA-256 digests for the trusted
   parent's durable receipt. Parent checks that each path remains in configured
   state scope and persists its own receipt/checkpoint transaction.

Parent may send `cancel` instead of any awaited response. Disconnect and absolute
deadline also fail closed. Parent should kill the owned process on cancellation,
timeout, malformed output or refusal; the deadline bounds any pending stdin read.

`response_format` describes model output:

| Value | Exact model response |
|---|---|
| `artifact` | JSON object with `artifact` and `trajectory`, both nonempty strings. Trajectory is explicit observed actions/results, not hidden reasoning. |
| `harness` | JSON object with `harness` (complete four-file string map) and `notes` (string). |
| `experience` | Upstream `<edits>...</edits>` and `<experience>` tags; `cue:`, `lesson:`, `evidence:` required, `confidence:` optional. |
| `selection` | Reserved for larger future budgets; current `k=1` uses upstream trivial selection. |

All model output is untrusted data. Its evidence/lesson text is an attributed
hypothesis; nonempty upstream fields do not verify truth. No instruction learned
in a tree can authorize tools, access credentials or change Policy/domain/lease
rules. The promotion callback must not accept an authority claim made by a model.

## Global budget and recovery

Each call reserves input tokens using prompt UTF-8 bytes plus 256 overhead tokens,
the per-call output ceiling and the parent-provided worst-case cost ceiling. It
must fit every remaining global ceiling before the request is emitted. The parent
must ensure this reserve covers its final transport rendering; otherwise refuse.
Known usage settles the reservation; unknown fields consume their full reserves.
The exposed upstream ledger retains method counts and labels. Its default USD
fields are omitted because only the parent's `usage` reports actual/reserved cost.
Retries, failed calls and reflection share the same totals and absolute deadline.
Above-reservation reports are recorded and cause terminal refusal. Actual runtime
limits are still enforced by the parent's model transport before spending.

Namespace identity separates authorization domain, task family, model/config
digest and upstream commit. An individual run identity additionally binds task,
evidence, base harness, Focus/objective/policy and limits. The absolute deadline
is refreshed on a trusted resume; it is not part of the content identity. A
completed exact match resumes without new calls. Changed input is refused, and a
failed or interrupted run is terminal under the same run/task ID. Use an explicit
new run ID for a new attempt; the parent controls aggregate spending across runs.

Full task prompts, stage contexts and trajectories stay in a private temporary
directory removed after execution. The persistent library contains complete
adopted trees, lessons, evidence references/digests and provenance; it does not
persist rendered stage prompts or Veil mappings. Data remains local to the trusted
state root (0700/0600 on POSIX). Secret/credential filtering belongs to Core/Vault
before any worker input; the worker cannot establish that arbitrary text is safe.

Library entries publish as whole directories, and become readable only when a
completed receipt commits their exact manifest digest. Orphaned entries from an
interrupted commit stay inert. Resume verifies adopted tree hashes. A same-domain
lesson is reloaded only when every supporting evidence reference **and digest**
is admitted again in the new request. Symlink ancestors, traversal IDs, stale
objective/policy identities and cross-domain reuse are refused. Library size is
bounded to 64 entries; exceeding the cap requires explicit operator retention,
not silent deletion of history.

Upstream final selection excludes baseline rollouts and cannot abstain. Therefore
`selection` records upstream adoption and `promotion` records Archie's separate
baseline/admission decision. Every result says `improvement_verified:false`.
