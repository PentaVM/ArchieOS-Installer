# Dataset plan for the Archie adapter

The trainable component should predict bounded advice: reference utility, clarification
need, model/tool choice and proposal structure. It must never learn the privacy gate,
issue a lease, decide what enters memory or execute a tool. Those remain deterministic
Rust boundaries.

## Recommended sources

| Need | Dataset | Use | Status and caveat |
|---|---|---|---|
| Function/tool proposal | [Berkeley Function Calling Leaderboard](https://gorilla.cs.berkeley.edu/leaderboard) | Schema selection, argument construction, multi-turn evaluation | Good evaluation source; keep its held-out sets out of training. Verify each released data license/revision before ingestion. |
| Software work | [SWE-bench](https://github.com/SWE-bench/SWE-bench) | Long task decomposition, file-reference selection, test-driven completion | Framework is MIT. Individual repository content retains its upstream license; record repo and commit provenance. |
| Desktop multimodality | [OSWorld V2](https://github.com/xlang-ai/OSWorld-V2) | Evaluation of long-horizon workstation tasks | Apache-2.0 code, but complete task assets are gated. Use as an evaluation environment, not an automatic training scrape. |
| Adversarial tool use | [AgentDojo](https://github.com/ethz-spylab/agentdojo) | Test indirect prompt injection and privacy/tool boundary failures | Evaluation only. Never optimize a learned allow/deny policy from it. |
| Clarification | [AmbigNQ](https://github.com/shmsw25/AmbigQA) | Detect multiple plausible interpretations and generate a minimal disambiguation | Useful bootstrap; verify the dataset terms and Wikipedia-derived content before redistribution. |
| Rule clarification | [OR-ShARC](https://github.com/Yifan-Gao/open_retrieval_conversational_machine_reading) | Learn when a missing condition changes the answer | Distributed under CC BY-SA 3.0; attribution/share-alike implications need review before mixing into weights. |
| Retrieval evaluation | [BEIR](https://github.com/beir-cellar/beir) | Evaluate context ranking across domains | BEIR explicitly says underlying dataset licenses remain the user's responsibility. Use selected subsets only after per-source review. |
| English/multilingual ASR | [Mozilla Common Voice](https://commonvoice.mozilla.org/en/datasets) | ASR robustness and accent coverage | Current dataset releases identify clips as CC0. Keep ASR training separate from Cerebra and preserve release datasheets. |
| Parametric CAD | [SketchGraphs](https://github.com/PrincetonLIPS/SketchGraphs) | Sketch constraint and construction-sequence priors | Code is MIT; original sketch creators retain copyright under Onshape terms. Do not train commercial weights until those terms are cleared. |
| CAD execution | Archie-generated FreeCAD fixtures | Tool proposal → policy → deterministic CAD execution | Preferred first training source: synthetic, reproducible, owned, and validated by solid/recompute/export checks. Mark every sample synthetic. |

ToolBench and other scraped API corpora are not approved by default. Their endpoint terms,
payload provenance and license must pass review before a manifest can mark them
`train_allowed`.

## Training split by component

1. **Context utility ranker.** Bootstrap on synthetic tasks that declare requested refs,
   useful refs, task success and privacy exposure. Then update online only from
   `CuratedTrainingRecord` produced by `admit_learning`. Persist digests and integer
   weights, never document text.
2. **Clarification estimator.** Train a small classifier/regressor over bounded permille
   targets using AmbigNQ/OR-ShARC transformations plus owned CAD scenarios. The trusted
   `ClarificationAssessment::decide` function remains outside the model.
3. **Planner adapter.** SFT or LoRA can learn to emit a strict proposal schema from BFCL,
   licensed SWE-bench trajectories and owned FreeCAD tasks. Replay every proposal through
   `gate_model_proposal`, policy and a disposable workspace.
4. **Floor estimator.** Start from public conversational audio only after a dataset has
   labels for hold/continue/end-of-turn and its voice-data terms are accepted. Runtime
   learning remains online weights-only until Nelson makes the open retention decision.
5. **ASR.** Treat Nemotron Speech adaptation as a separate model project. Common Voice
   can improve acoustic coverage, but it does not teach Cerebra autonomy or tool use.

## Admission manifest

Every imported release needs a pinned source revision, SHA-256, license/SPDX expression,
allowed purpose (`train`, `validation`, or `evaluation`), redistribution flag, PII review,
and transformation provenance. Unknown license, missing checksum or unknown PII status is
a hard admission failure. Raw professional/user data is never converted into a training
set merely because it is locally available.

Initial rollout should train on owned synthetic FreeCAD workflows and the existing
admitted utility signals, while keeping BFCL, OSWorld, AgentDojo and BEIR as untouched
evaluation sets. That gives a measurable adapter without teaching it to bypass the
kernel's authority or contaminating evaluation.

