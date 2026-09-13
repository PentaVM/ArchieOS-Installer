# Datasets for ArchieOS components

Surveyed 2026-09-13. Every Hugging Face entry below was checked the same day against the
Hugging Face API (`/api/datasets/<id>`, `/api/models/<id>`): the repository exists, and the
licence and gating shown are the ones in its card metadata. Entries marked *agent-fetched*
were read from their primary page by a research agent whose independent verification pass
did not complete; treat their licence as unconfirmed until someone re-reads the page.

A licence in card metadata is where to start, not a legal opinion. Several corpora keep the
licences of the audio they were built from, and that is noted where it applies.

## What this changes before anything is trained

**The planned ASR model does not speak Spanish.** The workstation plan names
`nvidia/nemotron-speech-streaming-en-0.6b`, which is English-only, and the operator speaks
Spanish. Its model card links a multilingual extension that is a direct replacement:

| Model | Licence | Verified on the card |
|---|---|---|
| `nvidia/nemotron-3.5-asr-streaming-0.6b` | OpenMDW-1.1 | "multilingual extension of nemotron-speech-streaming-en-0.6b"; 40 language-locales from one model; cache-aware streaming; "ready for commercial use"; evaluated on FLEURS Spanish |
| `nvidia/parakeet-tdt-0.6b-v3` | CC-BY-4.0 | 25 European languages including Spanish; chunked rather than cache-aware streaming (*agent-fetched details*) |

**Public data trains perception only until the operator decides otherwise.** Archie retains
no household audio (RFC-011). Every speech dataset here contains real people's voices, so
consent basis and licence both matter.

## ASR — `archie-speech`

| Dataset | Licence (card) | Commercial | Access | Notes |
|---|---|---|---|---|
| `nvidia/Granary` | cc-by-4.0 | yes | open | ~643k h pseudo-labelled, 25 European languages incl. Spanish. **Manifests only** — audio comes from YODAS, YouTube-Commons, VoxPopuli and LibriLight under their own terms. Machine labels: do not use as an evaluation reference. |
| `facebook/multilingual_librispeech` | cc-by-4.0 | yes | open | ~918 h Spanish read audiobooks. Also in Nemotron's training set, so MLS test scores are in-domain. |
| `google/fleurs` | cc-by-4.0 | yes | open | Evaluation. Spanish is `es_419` only. Also in Nemotron's training data — expect optimistic scores. |
| `facebook/voxpopuli` | cc0-1.0 + other | unclear | open | European Parliament speech; the "other" refers to EP terms. |
| Common Voice 26.0 Spanish (Mozilla Data Collective) | CC0-1.0 | yes | click-through | *agent-fetched.* 2,279 h recorded, 26,940 speakers, wide accent coverage. Since October 2025 distributed only via Mozilla Data Collective; the HF mirrors are empty, so `load_dataset` pipelines break. |

## Local TTS — not present in Archie today

| Dataset | Licence (card) | Commercial | Notes |
|---|---|---|---|
| `nvidia/hifitts-2` | cc-by-4.0 | yes | English. Manifests only. |
| `ylacombe/cml-tts` | cc-by-4.0 | yes | Spanish among seven languages; audiobook style. |
| LibriTTS-R (OpenSLR 141) | CC BY 4.0 | yes | *agent-fetched.* English. |
| OpenSLR 73, Peruvian Spanish | CC BY-SA 4.0 | yes | *agent-fetched.* Regional, small. |

Gap: no permissive *conversational* Spanish TTS corpus was found.

## Floor estimation — `archie-memory::floor` inputs

| Dataset | Licence (card) | Commercial | Access | Notes |
|---|---|---|---|---|
| `edinburghcstr/ami` | cc-by-4.0 | yes | open | English meetings with word timing: overlaps, pauses, turn ends. |
| `diarizers-community/voxconverse` | cc-by-4.0 | yes | open | Speaker-change timing for diarization. |
| `talkbank/callhome` | **cc-by-nc-sa-4.0** | **no** | gated | Telephone conversation. Research only. |
| Fisher Spanish (LDC2010S01) | LDC agreement | paid | paid | *agent-fetched.* The only verified conversational Spanish corpus; 8 kHz telephone. |

Gap: no permissive Spanish turn-taking corpus exists in this survey. Timing and prosody
features (pause length, pitch reset, overlap) transfer across languages far better than
lexical ones, which argues for training the estimator on those features from AMI and
VoxConverse, then calibrating locally.

## Clarification — when to ask versus proceed

| Dataset | Licence (card) | Commercial | Notes |
|---|---|---|---|
| `nvidia/When2Call` | cc-by-4.0 | yes | Directly the decision the clarification engine makes: call a tool, ask, or answer. |
| `sewon/ambig_qa` | cc-by-sa-3.0 | yes (share-alike) | Ambiguous questions with their interpretations. |
| τ²-bench (`sierra-research/tau2-bench`) | MIT | yes | *agent-fetched.* Dual-control agent tasks; evaluation. |

## Tool use — `archie-mcp` gate evaluation

| Dataset | Licence (card) | Commercial | Access |
|---|---|---|---|
| `nvidia/Nemotron-Agentic-v1` | cc-by-4.0 | yes | open |
| `nvidia/Nemotron-SFT-Agentic-v2` | cc-by-4.0 / apache-2.0 / mit (per subset) | yes | open |
| `nvidia/Nemotron-RL-agent-workplace_assistant` | cc-by-4.0 | yes | open |
| `gorilla-llm/Berkeley-Function-Calling-Leaderboard` | apache-2.0 | yes | open |
| `Salesforce/xlam-function-calling-60k` | cc-by-4.0 | check gate terms | gated |
| `AmazonScience/massive` | cc-by-4.0 | yes | open — includes `es-ES` intents |

## Prompt injection and safety

| Dataset | Licence (card) | Commercial | Notes |
|---|---|---|---|
| `nvidia/Nemotron-RL-Agentic-Indirect-Prompt-Injection-v1` | cc-by-4.0 | yes | Injection through tool output — the attack on capsule content and MCP results. |
| `nvidia/Nemotron-Safety-Guard-Dataset-v3` | cc-by-4.0 | yes | Multilingual, includes Spanish. |
| AgentDojo (`ethz-spylab/agentdojo`) | MIT | yes | *agent-fetched.* Injection benchmark for tool-using agents. |
| `nvidia/Nemotron-AIQ-Agentic-Safety-Dataset-1.0` | other (NVIDIA Evaluation Dataset License) | **no** | Evaluation only. |

## CAD — FreeCAD / parametric modelling

| Dataset | Licence (card) | Commercial |
|---|---|---|
| `SadilKhan/Text2CAD` | **cc-by-nc-sa-4.0** (gated) | **no** |
| `filapro/cad-recode` | **cc-by-nc-4.0** | **no** |

Both are non-commercial: acceptable for internal evaluation, not for training a model that
ships. No permissively licensed CAD sequence dataset was verified.

## Retrieval — the stage-2 context-utility learner

| Dataset | Licence (card) | Commercial | Notes |
|---|---|---|---|
| `hotpotqa/hotpot_qa` | cc-by-sa-4.0 | yes (share-alike) | Multi-hop: which passages were actually needed. |
| `BeIR/scifact` | cc-by-sa-4.0 | yes (share-alike) | One BEIR task; each BEIR set carries its own licence. |
| `microsoft/ms_marco` | **not stated in card metadata** | check | Read the MS MARCO terms before any training use. |

The stage-2 learner should train mainly on Archie's own admitted outcomes, which is what it
is built for; public retrieval sets are for evaluation and bootstrapping.

## PII detection — `archie-vault` pseudonymisation

| Dataset | Licence (card) | Commercial | Notes |
|---|---|---|---|
| `nvidia/Nemotron-PII` | cc-by-4.0 | yes | |
| `gretelai/synthetic_pii_finance_multilingual` | apache-2.0 | yes | Synthetic, multilingual. |
| `ai4privacy/pii-masking-400k` | other (custom) | read the licence | |
| `eriktks/conll2002` | unknown in card | check | Spanish NER. |

## Verification record

All 29 Hugging Face datasets and four of five model identifiers resolved on 2026-09-13.
`nvidia/nemotron-3-super-120b-a12b` — the name Sheriff's integration uses — does not
resolve on Hugging Face (401). It is a Nebius Token Factory model name, not a public
Hugging Face repository; the public checkpoint is `nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-FP8`
(licence: other).
