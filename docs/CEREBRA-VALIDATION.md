# Cerebra stages 1–3: executed validation

Date: 2026-09-13. Windows x86_64, `rustc 1.98.0 (88d9e12ae 2026-08-18)`.
Starting main: `6a2454c0a5dd43d7c03cf4ae3ca6decac38996bb`.

Every stage ran `cargo fmt --all --check`, crate-scoped clippy with `--all-targets
-- -D warnings`, and its tests plus Cerebra, memory and observability. Incremental
compilation was disabled; dev/test debug symbols were disabled to conserve C: space.
`df -h /` was checked before builds. The historical `Desktop/Archie` was not used.

| Gate | Passed tests (including compile-fail doctests) | Failed |
|---|---:|---:|
| Stage 1, `721e873` | 263 | 0 |
| Stage 2, `73170f6` | 274 | 0 |
| Stage 3, `96a8a57` | 371 | 0 |
| Final, with redirect and complete-evidence checks | 411 | 0 |

Final commands, from `core/`:

```powershell
$env:CARGO_INCREMENTAL='0'
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_PROFILE_TEST_DEBUG='0'
cargo fmt --all --check
cargo clippy -p archie-cerebra -p archie-memory -p archie-observability -p archie-speech -p archie-model-runtime --all-targets -- -D warnings
cargo test -p archie-cerebra -p archie-memory -p archie-observability -p archie-speech -p archie-model-runtime -p archie-vault
```

| Final crate | Passed |
|---|---:|
| archie-cerebra | 35 |
| archie-memory | 133 |
| archie-observability | 113 |
| archie-speech | 95 |
| archie-model-runtime | 9 |
| archie-vault | 26 |

Specific witnesses:

- 16 advisory tests: strict parsing, citations, uncertainty, contradiction, ambiguity,
  real loopback NIM-compatible HTTP, oversized responses, explicit trace codes, capsule
  expiry and complete admitted evidence. Settled assessments produce zero connections.
- All 12 original `loop_order` tests pass. The floor implementation has no diff from
  the starting commit; its four memory tests and 20 speech policy tests pass.
- Eight utility tests, two gateway/checkpoint integration tests and one compile-fail
  admission test. They cover subset-only retrieval, cold start, real exposure cost,
  reproducibility, altered rewards, rejected admission, replay, corruption and expiry.
- Eight estimator tests, two Cerebra/checkpoint integration tests and three compile-fail
  feature/label tests. Numeric synthetic buffers are generated transiently and erased;
  no recordings or transcripts are fixtures. Online updates change only weights.
- `cargo tree -p archie-speech -e normal` contains no `reqwest`, `hyper`, `tokio`,
  `archie-model-runtime` or `archie-router`. Cloud/Fabric construction and restore fail.
- A two-listener HTTP 307 test observes zero connections to the redirect target.

Limits: the existing proposal gate abstains throughout the ambiguous band, so advisory
acceptance is not claimed. Contradiction and uncertainty reasons that require settled
evidence are tested directly against the parser/gate, never by sending settled evidence
over HTTP. No live provider account, real labelled dataset, accuracy, microphone driver
integration, automated promotion or background checkpoint-file cleanup is claimed.
Encrypted checkpoint access expires after seven days; the operator owns physical
cleanup. Retaining any sensory corpus remains Nelson's separate decision.
