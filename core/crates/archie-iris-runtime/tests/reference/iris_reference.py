"""Reference outputs of the Python IRIS (archie_iris_foundry) on CPU, float32, for the Rust parity test.

Test tooling only: it runs the original Python model so `tests/parity.rs` and
`tests/text_tower.rs` have something to agree with. Nothing at run time uses it.

    PYTHONPATH=<archie-iris>/src python iris_reference.py <weights.pt> <image.png> <vjepa2 clone at 204698b> <out dir>

Offline: V-JEPA comes from a local clone of facebookresearch/vjepa2 at the pinned revision,
SigLIP2 is built from its config (the vision tower is then overwritten by the release's
weights), and phrase features are deterministic stand-ins because the frozen text tower is
not in the release and Hugging Face is unreachable here. Rust is fed the same features.
"""
import hashlib, json, sys, time
from pathlib import Path

import numpy as np
import torch
from safetensors.torch import save_file

weights, image_path, vjepa_repo, out_dir = map(Path, sys.argv[1:5])
out_dir.mkdir(parents=True, exist_ok=True)
torch.manual_seed(0)
torch.set_num_threads(4)

# --- offline construction ---------------------------------------------------------------
_hub_load = torch.hub.load
def local_hub(repo, model, *args, **kwargs):
    kwargs.pop("trust_repo", None); kwargs.pop("skip_validation", None)
    return _hub_load(str(vjepa_repo), model, *args, source="local", **kwargs)
torch.hub.load = local_hub

from transformers import SiglipConfig, SiglipModel
import archie_iris_foundry.beta_model as bm

def offline_siglip(repository, revision, activation_checkpointing):
    key = (repository, revision)
    if key not in bm._SIGLIP_MODELS:
        config = SiglipConfig(
            text_config=dict(vocab_size=256000, hidden_size=768, intermediate_size=3072, num_hidden_layers=12,
                             num_attention_heads=12, max_position_embeddings=64, hidden_act="gelu_pytorch_tanh",
                             layer_norm_eps=1e-6, projection_size=768),
            vision_config=dict(hidden_size=768, intermediate_size=3072, num_hidden_layers=12, num_attention_heads=12,
                               image_size=384, patch_size=16, hidden_act="gelu_pytorch_tanh", layer_norm_eps=1e-6))
        bm._SIGLIP_MODELS[key] = SiglipModel(config).eval()
    return bm._SIGLIP_MODELS[key]
bm.load_siglip = offline_siglip

import transformers
_auto = transformers.AutoConfig.from_pretrained
class _Dino:  # dinov2-base: only the teacher's width is read in inference
    hidden_size = 768
transformers.AutoTokenizer.from_pretrained = lambda *a, **k: None  # phrases are injected below
transformers.AutoConfig.from_pretrained = lambda name, *a, **k: _Dino() if "dinov2" in name else _auto(name, *a, **k)

def phrase_features(text, width=768, limit=32):
    seed = int.from_bytes(hashlib.sha256(text.encode()).digest()[:8], "little")
    generator = torch.Generator().manual_seed(seed)
    count = 1 + seed % 6
    pooled = torch.randn(width, generator=generator).to(torch.float16).float()
    states = (torch.randn(min(count, limit), width, generator=generator) * 0.5).to(torch.float16).float()
    return pooled, states

USED = {}
def fake_encode(self, phrases, device):
    values = []
    for text in phrases:
        pooled, states = phrase_features(text)
        USED[text] = (pooled, states)
        values.append((pooled, states))
    counts = [int(s.shape[0]) for _, s in values]
    width = max(counts, default=1)
    tokens = torch.zeros(len(values), max(1, width), 768)
    for row, (_, states) in enumerate(values):
        tokens[row, :states.shape[0]] = states
    pooled = torch.stack([p for p, _ in values]) if values else torch.zeros(0, 768)
    return pooled.to(device), tokens.to(device), counts
bm.PhraseEncoder.encode = fake_encode

from archie_iris_foundry.iris_infer import IrisDetector, EVERYDAY_VOCABULARY
from archie_iris_foundry.beta_data import letterbox
from PIL import Image

started = time.time()
detector = IrisDetector.from_weights(weights, "cpu")
print("loaded", round(time.time() - started, 1), "s", detector.info["parameters"], flush=True)
model = detector.model

with Image.open(image_path) as image:
    image = image.convert("RGB")
array = np.asarray(image)
canvas, _, _ = letterbox(array, np.zeros((0, 4), np.float32), np.zeros((0, 2), np.float32), model.config.image_size)
tensor = torch.from_numpy(canvas).permute(2, 0, 1).unsqueeze(0).float().div(255.0)

tensors = {"canvas": torch.from_numpy(canvas.copy())}
with torch.no_grad():
    t = time.time()
    vjepa = model.vjepa(tensor); siglip = model.siglip(tensor)
    tokens = model.vjepa_projection(vjepa) + model.siglip_projection(siglip) + model.position
    print("encode", round(time.time() - t, 2), "s", flush=True)
    tensors.update(vjepa=vjepa[0], siglip=siglip[0], tokens=tokens[0])
    phrases = ["button", "person", "window title", "text", "icon", "menu"]
    t = time.time()
    out = model.forward_tokens(tokens, [phrases])
    print("phrases", round(time.time() - t, 2), "s", flush=True)
    tensors.update(fused=out["fused"][0], logits=out["logits"][0], boxes=out["boxes"][0],
                   token_logits=out["token_logits"][0], token_boxes=out["token_boxes"][0],
                   instance_embedding=out["instance_embedding"][0])

report = {"image_size": [image.width, image.height], "phrases": phrases}
report["detect"] = [hit.to_dict() for hit in detector.detect(image, phrases, threshold=0.05, with_embeddings=True)]
t = time.time()
report["everything"] = [hit.to_dict() for hit in detector.detect_everything(image, threshold=0.05, extra=["red button"], with_embeddings=True)]
print("everything", round(time.time() - t, 2), "s", flush=True)

from archie_iris_foundry.iris_act import IrisActor
actor = IrisActor(model, torch.device("cpu"))
controls = []
for step, (instruction, now) in enumerate([("click the search field", 10.0), ("click the search field", 10.6),
                                            ("type hello", 11.3)]):
    control = actor.act("parity", array, instruction, skill="browser", now=now)
    control.pop("latency_ms"); control.pop("timing_ms")
    controls.append(control)
report["act"] = controls

names = sorted(USED)
for index, name in enumerate(names):
    tensors[f"phrase.{index}.pooled"] = USED[name][0]
    tensors[f"phrase.{index}.states"] = USED[name][1]
(out_dir / "phrases.json").write_text(json.dumps(names, ensure_ascii=False))
save_file({k: v.contiguous() for k, v in tensors.items()}, str(out_dir / "reference.safetensors"))
(out_dir / "reference.json").write_text(json.dumps(report, ensure_ascii=False, indent=1))

# The text tower on its own: a random SigLIP text model, fixed ids, with and without a mask.
text = bm._SIGLIP_MODELS[(model.config.siglip_repository, model.config.siglip_revision)].text_model
ids = torch.randint(1, 256000, (3, 64), generator=torch.Generator().manual_seed(7))
ids[0, 5:] = 0; ids[1, 20:] = 0
mask = (ids != 0).long()
with torch.no_grad():
    plain = text(input_ids=ids)
    masked = text(input_ids=ids, attention_mask=mask)
save_file({**{f"text_model.{k}": v.contiguous() for k, v in text.state_dict().items()}},
          str(out_dir / "text_tower_random.safetensors"))
save_file({"ids": ids.to(torch.int64), "mask": mask, "plain_pooled": plain.pooler_output,
           "plain_states": plain.last_hidden_state, "masked_pooled": masked.pooler_output,
           "masked_states": masked.last_hidden_state}, str(out_dir / "text_reference.safetensors"))
print("done", round(time.time() - started, 1), "s", flush=True)
