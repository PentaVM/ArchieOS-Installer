# Reference outputs of the real SigLIP2 text tower (google/siglip2-base-patch16-384 @ f775b65)
# from transformers, for archie-iris-runtime's tests/text_tower.rs (real_tower).
import json, sys, torch
from transformers import AutoTokenizer, SiglipModel
from safetensors.torch import save_file
tower_dir, out = sys.argv[1], sys.argv[2]
phrases = ["button", "person", "window title", "Cerrar sesión", "the red cup on the left",
           "taza de café", "ícono de búsqueda", "air conditioner", "", "Save As…"]
tok = AutoTokenizer.from_pretrained(tower_dir)
lowered = [p.lower() for p in phrases]
enc = tok(lowered, padding="max_length", max_length=64, truncation=True, return_tensors="pt")
model = SiglipModel.from_pretrained(tower_dir, torch_dtype=torch.float32).eval().text_model
with torch.no_grad():
    o = model(input_ids=enc["input_ids"])
save_file({"ids": enc["input_ids"].to(torch.int64), "pooled": o.pooler_output.contiguous(),
           "states": o.last_hidden_state.contiguous()}, f"{out}/real_tower.safetensors")
json.dump({"phrases": phrases, "attention_mask_returned": "attention_mask" in enc,
           "pad_id": tok.pad_token_id}, open(f"{out}/real_tower.json", "w"), ensure_ascii=False)
print("ok", enc["input_ids"][:3, :8].tolist(), tok.pad_token_id)
