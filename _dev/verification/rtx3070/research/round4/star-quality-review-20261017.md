# STaR data-quality review — 2026-10-17 21:25 UTC

Advice for Claude, main developer. Research/review used **gpt-6.1-sol medium**
in the same chat, followed by root's independent pure-Python reproductions.
Origin verified: `PentaVM/ArchieOS-Nvidia`; checkout HEAD `9183b8e` plus Claude's
uncommitted STaR changes. No active trainer, queue, control, hold, watchdog,
thermal configuration or model was changed. No native module/tokenizer was
imported, no GPU job was started, and no new training labels were authored.

## Findings to resolve before treating g3 as a clean training tranche

1. **P1: a matching prefix is not a correct final answer.** `star.py:26–30`
   accepts gold `no` for both `No is incorrect; yes is the answer.` and
   `No. Correction: yes.`. The builder at line75 discards this prose and installs
   the gold label after the trace. Thus an accepted row can hide the original
   incorrect/contradictory answer. The independent reviewer also reproduced
   `Yes or no cannot be determined.` for gold `yes`. Current word-boundary tests
   catch `Nobody`, but not these cases. Prefer an unambiguous answer grammar;
   quarantine contradictions and multiple labels. Keep the raw answer and its
   parse result in the curation receipt. Do not rewrite gold to improve scores.

2. **P1: natural completion needs affirmative stop evidence.**
   `NativeChat.think` receives the delimiter-stop boolean, but derives
   `forced_end` solely from `used >= budget` (`native_chat.py:98–109`). A
   non-delimiter early stop with ten reasoning tokens followed by a matching
   answer is marked unforced and accepted by the current filter. A pure
   completion fixture reproduces this path without loading the runtime.
   Preserve the actual stop reason/word and distinguish the intended closing
   delimiter, EOS, context limit, token limit and errors. Test the exact-budget
   delimiter boundary separately. This is a code-path finding; no incidence of
   this failure in the live native run has been measured.

3. **P2: resume identity is weaker than provenance.** `worker.py:415–419`
   reuses JSONL entries by example ID without binding the source bytes, rendered
   prompt, model/checkpoint, generation/hint settings or code version. The
   tranche records a source path, not an immutable generation manifest. A new
   source with the same IDs or changed budgets can reuse old traces under new
   metadata. The LOG explicitly says the old640 records were set aside; that
   manual separation is useful but is not enforced by the resume contract.
   Freeze hashes/settings at run creation and reject mismatched resumes, or
   use a new run ID. Preserve `trace: self|rationalized` through preparation:
   `worker.py:260–266` currently omits it from prepared example records.

4. **P2: cap acceptance is not full-sequence admission.** 840 thinking tokens
   leave only184 for the prompt, framing, answer and delimiters in a1024-token
   sequence. Prompt lengths vary. The later extractor/prepare step does reject
   absent receipts, so this review does **not** claim silent truncation.
   Report both label-filter acceptance and full rendered-sequence admission;
   bind the accepted IDs and rejection reasons before comparison. Token fit is
   not measured by this CPU audit, which loads no native tokenizer.

5. **P2: lexical hint exclusion is a heuristic.** The current filter accepts
   `The supplied solution says no; I will justify that conclusion.` but rejects
   the ordinary Spanish word `despistada` through the substring `pista`.
   Use language-aware boundaries and sampled semantic review, preserving the
   raw traces and rejection reasons. Removing explicit hint phrases cannot
   certify causal correctness or independent reasoning.

The source also admits missing completion/token-count fields and negative/NaN
counts. The audit includes these defensive-schema cases, but the bounded live
sample had all such fields; it does not establish real malformed records.
Require explicit completion evidence and finite nonnegative integer counts.

## Reproducible CPU evidence

Run from the Wanda checkout:

```powershell
python _dev/verification/rtx3070/research/round4/audit_star_acceptance.py --source training/wanda_bonsai2/src/archie_wanda/star.py --out _dev/verification/rtx3070/research/round4/star-acceptance-audit.json
```

The script AST-extracts the exact functions instead of importing Wanda.
Source SHA256 `2803dcf3dd313e70b0793284e45c528544189a9abb4c187670b214ab801c0a62`.
Receipt:13 cases, six adversarial mismatches (two contradictory answers and
four malformed-evidence cases), plus the lexical-filter examples and a separate
early-stop fixture. These are deliberately constructed admission tests, not
training items, an estimate of live error rates, hardware proof or model scores.

Read-only system-Python metadata snapshot at21:25:28UTC found workerPID6290
paused on `bench-fantom-base`, progress7209/12832. The current STaR response
file contained30 records,24 forced ends, mean951.8 thinking tokens, with no
missing completion/count fields. The separate old-budget file contained28
records,26 forced ends, mean623.82. No rationale JSONL was present in the
bounded newest-eight-file scan. These observations support the reported low
self-trace yield and ongoing FANToM work; they do not diagnose the pause or
prove GPU availability. `live_metadata_cpu.py` reads bounded metadata only,
writes nothing in WSL, and emits no prompts, answers or benchmark content.

## Useful next comparisons, without adding volume

The original [STaR paper](https://arxiv.org/abs/2203.14465) explicitly includes
retrying failed questions with the correct answer supplied. Claude's use of
rationalization is therefore methodologically recognizable; the hint itself is
not grounds to invalidate the method. The new issue is the strength of the
acceptance and provenance claims. The [authors' primary summary](https://research.google/pubs/star-self-taught-reasoner-bootstrapping-reasoning-with-reasoning/)
also describes this correct-answer-conditioned retry. Neither source establishes
that every retained rationale is causally faithful or that it transfers to Wanda.

After the admission fixes are reviewed by Claude, compare g2 alone, g2+self-only,
g2+rationalized-only and g2+mixture. Hold optimizer steps, accepted target-token
exposure, base/checkpoint, rank/alpha, seeds17/29, answer weighting and evaluation
reasoning budget fixed; separately report trace-generation cost. If a self-only
cohort is too small, report that limitation rather than repeating it to create
nominal diversity. Compare both a common eligible family set and full eligible
sets to expose selection bias from the short-trace cap. Freeze TRAIN provenance;
keep development and evaluation family-disjoint and entirely hint-free.

Record accepted/rejected counts by generator mechanism, label, language, causal
pair, trace source and rendered-length bucket. Preserve both sides of a causal
pair or disclose pair loss; do not select favorable external benchmark items.
Use paired narrative/world-level uncertainty for per-type deltas; questions in
one world are not independent replicates. A lexical clean bill and two positive
seed point estimates are not statistical evidence of social generalization.

Claude's LOG now reports g2 direct OpenToM improvements in both seeds with zero
unparseables; this is later work on a new corpus/protocol, separate from the old
anchor's unchanged-choice result. The1024-thinking pilot is mixed, and FANToM
is still in progress. This review inspected code/metadata, not the complete
OpenToM prediction files, so it does not independently certify those scores.
Do not promote an adapter from training loss, a parser change or partial results.

Existing round2/round3 candidates remain20 TRAIN families/80 variants,
pending-human; this review adds no corpus rows. Core/Cortex, six-field output,
personal/work boundaries and Claude/Xavi ownership remain unchanged. Our own
fine-tuning stays deferred while Claude's benchmark owns the execution workflow;
old strict-prefill/backward receipts do not substitute for an admitted semantic
run, native-export verification or a new blind benchmark. No merge requested.
