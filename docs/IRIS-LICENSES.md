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

## What the WSL Operator image carries

- **IRIS Beta 2.1 weights** (`iris-beta21-step132000.pt`, SHA-256 `c6cadb02…3b97`), from
  the archie-iris release, trained as non-commercial research. Their `LICENSE` (and model
  card, when given) is installed beside them in `/var/lib/archie/models/iris/`.
- **SigLIP2's text tower**, cut from `google/siglip2-base-patch16-384` at revision
  `f775b65a79762255128c981547af89addcfe0f88` (Apache-2.0; `model.safetensors` SHA-256
  `ed72c0ac…0857`). Only the `text_model.*` tensors and the tokenizer files are kept, with
  a `SOURCE` file naming what they were cut from.
- **The Rust runtime** (`archie-iris-serve`), which is ArchieOS's own code, and, in the
  CUDA build, NVIDIA's cuBLAS, cuBLASLt, cuRAND and NVRTC runtime libraries, which the
  CUDA Toolkit EULA allows applications to redistribute.

No V-JEPA code, PyTorch or Hugging Face download is involved at run time any more.
