# IRIS privacy

Raw frames are bounded, zeroed on drop, non-serializable and absent from traces and durable
memory. Only semantic references and model weights may persist. Remote providers receive
only task-selected, policy-cleared material; local-only and Secret inputs cannot cross that
boundary. Biometrics are local-only.

Training accepts registry-admitted public/synthetic sources or records that passed the
existing learning-admission firewall. Retaining labelled private screen, camera, face or
audio examples requires Nelson's unresolved retention decision; the current system retains
none.

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
