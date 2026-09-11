# Cortex context request compatibility

Frames retain the existing length-prefix transport. Integer fields are big endian;
strings are UTF-8 bytes preceded by a u32 length. The first bytes are version and
message type (ContextRequest = 2). Graph, response and refusal frames remain v1.

All request versions begin with the v1 body: request ID, question, active project ID,
snapshot ID, sensitivity count and u8 values, u32 token budget, u16 maximum items,
u8 expansion hops and i64 request timestamp.

| Request version | Extension after the common body |
| --- | --- |
| 1 | None; exact original framing retained |
| 2 | Required Focus body |
| 3 | u8 Focus-present flag (0/1), optional Focus body, u32 cue count, cue bodies |

The Focus body is Focus ID, u8 suspended flag, i64 half-life, u32 activation count,
then node ID, u32 strength in millionths, i64 activation timestamp and u8 pinned
flag for each activation. Maximum 128 activations; strengths are in [0, 1,000,000].

A cue body is a grounding node ID and a question (maximum 256 bytes). At most eight
unique grounding/question pairs are allowed. A cue activates only when its grounding
node is already on the policy-visible candidate frontier. Two synchronous rounds
bound prospective expansion independently of the existing zero-to-three graph hops.
Grounding paths must fit the final context budget together with their discoveries.
Cues are proposed information needs; they never create a fact or relationship.

Encoders select v3 when cues are present, v2 for Focus alone, v1 otherwise. Decoders
reject invalid flags, bounds, duplicates, trailing bytes and truncated bodies. Older
daemons refuse unsupported request versions; callers may explicitly retry without
cues/Focus if their policy permits degraded retrieval. There is no silent downgrade.

Rust and C++ tests consume the same fixtures under `protocols/fixtures`. The v1
graph and v2 Focus fixtures remain unchanged; v3 has a separate prospective fixture.
