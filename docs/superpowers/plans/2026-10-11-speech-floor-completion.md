# Speech floor completion implementation plan

> Execute inline using superpowers:executing-plans. The owner authorized autonomous completion, integration and publication; no repeat permission gate.

**Goal:** Finish the existing voice floor and consolidate outstanding work in Nvidia main.

**Architecture:** Existing FloorPolicy and SpeechSession remain authoritative. An ASR interface wraps the current SDK and permits deterministic replay of the same conversation loop. No GUI/WSL work.

**Tech Stack:** Rust, existing NeMo ABI, Piper/Magpie/RVC, existing channels and adapters.

**Spec:** ../specs/2026-10-11-speech-floor-completion.md

## Global constraints

- Work only in ArchieOS-Nvidia; preserve other worktrees and authority contracts.
- Native ASR and voice models remain unchanged; voice evidence is not identity.
- No model/dependency installation or GUI/WSL generation.
- Tests cannot certify measured latency or accuracy on real hardware.

## Review focus

- Empty/interim ASR versus acknowledgement: avoid cutting off on a listening noise.
- Final before physical silence: retain a clause until measured floor is available.
- Correction during old playback: retain it and invalidate old receipts.
- Shutdown/TTS error: queued speech is cancelled and never claimed as heard.
- Bilingual synthesis: preserve RVC path and fixed output sample rate.

## Tasks

- [x] Inventory branches/worktrees; fast-forward main to committed Sentry branch. Preserve staged WSL work.
- [x] Complete pending bilingual patch in piper.rs, language.rs and CLI; preserve current RVC branch. Test language selection and sample-rate validation.
- [x] Add ASR boundary in asr.rs, adapt native.rs and conversation.rs. Test the actual loop with scripted audio recognition and existing cognitive adapter.
- [x] Regress and fix recognition grace, held clauses, correction retention, generation receipts, cancellation and playback completion. Test no unrelated speech/tool execution.
- [ ] Run speech/voice and affected Sentry/Core/CLI/Workstation suites, validate Linux native features when SDK is available, inspect diffs and update handoff/bitácora. Fetch, integrate concurrent commits normally, push and compare remote SHA.
