# IRIS license policy

Every registry entry declares repository, immutable revision, intended split, license and
allowed use. Unknown licenses hard-fail before streaming or training. Evaluation-only
sources cannot enter training. Generated artifacts record the admitted dataset IDs and
checkpoint lineage; changing a source requires a new review and revision pin.

## LocateAnything-3B

The ArchieOS repository contains only an independently written API adapter and synthetic
wire-contract tests. It does not contain NVIDIA weights, model code, training data or
model outputs used as labels. NVIDIA describes `nvidia/LocateAnything-3B` as research and
development only and restricts its released weights to academic and non-profit research.
Commercial use is not permitted by that model license. The reviewed license revision is
`fb9f8b10ea275ea341e02cd2d5ab0f237b6afb80`:
<https://huggingface.co/nvidia/LocateAnything-3B/blob/fb9f8b10ea275ea341e02cd2d5ab0f237b6afb80/LICENSE>.

Consequently LocateAnything may be configured for an authorized hackathon research demo,
but it is not an admissible production dependency or teacher for commercial ArchieOS
training. Selecting a production grounding model requires a separate license review.
