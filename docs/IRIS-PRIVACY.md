# IRIS privacy

Raw frames are bounded, zeroed on drop, non-serializable and absent from traces and durable
memory. Only semantic references and model weights may persist. Remote providers receive
only task-selected, policy-cleared material; local-only and Secret inputs cannot cross that
boundary. Biometrics are local-only.

Training accepts registry-admitted public/synthetic sources or records that passed the
existing learning-admission firewall. Retaining labelled private screen, camera, face or
audio examples requires Nelson's unresolved retention decision; the current system retains
none.

**Skill learning (Nelson, 2026-09-28).** With the person's opt-in, and only then, Archie may
keep the day's own sessions to train a skill adapter (LoRA) during its REM sleep:
- stored only on the machine, encrypted in Vault, never uploaded as frames;
- excluding apps on the exclusion list (banking, password managers, messaging) and password
  fields, with faces blacked out;
- deleted at the end of each REM cycle;
- the reading (OCR) head never trains on these sessions, since a character decoder can
  memorise strings it sees often;
- if the person also opts in to community learning, only the adapter's weight change leaves
  the machine, clipped and noised (differential privacy), never frames or text.
A skill is promoted, locally or in the cloud, only when a sealed benchmark the person does not
see says it is better.

The LocateAnything boundary sends only a task-requested ROI whose dimensions and SHA-256
match the Cortex request. Its text must originate from a destination-bound Vault capsule;
credential references are refused. The encoded image exists only in the transient HTTP
body and is never serializable through the request type, traced or persisted. The runtime
repeats provider, locality, privacy and retention checks immediately before egress and
does not follow redirects.

The live Nebius smoke uses only a programmatically generated public image and public target
description. The API key is injected through the process environment for the call and is
not stored in a fixture, request body, trace or test output. Private and Restricted capsules
remain ineligible for this cloud route.
