"""Reference outputs of IRIS's reading head (archie_iris_foundry.beta_model.ReadDecoder) for the
Rust parity test `tests/read.rs`. Test tooling only.

    PYTHONPATH=<archie-iris>/src python read_reference.py <weights.pt> <out.safetensors>

Only the reading head is built: its weights come from the release, its memory is a fixed
random scene state, so the comparison isolates the decoder (teacher-forced logits and the
greedy generation, token by token). Runs on the CPU in float32.
"""
import sys

import torch
from safetensors.torch import save_file

from archie_iris_foundry.beta_model import READ_EOS, READ_PAD, ReadDecoder, encode_text

weights, out = sys.argv[1:3]
release = torch.load(weights, map_location="cpu", mmap=True, weights_only=True)
config = release["model_config"]
state = {key[len("read_decoder."):]: value.float() for key, value in release["model"].items()
         if key.startswith("read_decoder.")}
decoder = ReadDecoder(config["dimension"], config["heads"], config["read_max_length"])
decoder.load_state_dict(state)
decoder.eval()

torch.manual_seed(0)
memory = torch.randn(3, 64, config["dimension"])
texts = ["Hola, ¿qué tal?", "ArchieOS", "x"]
length = max(len(encode_text(t, config["read_max_length"])) for t in texts)
tokens = torch.full((len(texts), length), READ_PAD, dtype=torch.long)
for row, text in enumerate(texts):
    ids = encode_text(text, config["read_max_length"])
    tokens[row, :len(ids)] = torch.tensor(ids)

with torch.no_grad():
    # Teacher forcing over the real tokens only: padding is masked in Python and simply
    # absent in the comparison, so compare the first row (no padding) in full.
    logits = decoder(tokens[:1], memory[:1])
    generated = torch.full((len(texts), 1), 1, dtype=torch.long)
    finished = torch.zeros(len(texts), dtype=torch.bool)
    for _ in range(decoder.max_length - 1):
        step = decoder(generated, memory)[:, -1].argmax(-1)
        step = torch.where(finished, torch.full_like(step, READ_PAD), step)
        generated = torch.cat((generated, step.unsqueeze(1)), dim=1)
        finished |= step == READ_EOS
        if bool(finished.all()):
            break

save_file({
    "memory": memory.contiguous(),
    "tokens": tokens[:1].to(torch.int64).contiguous(),
    "logits": logits.contiguous(),
    "generated": generated[:, 1:].to(torch.int64).contiguous(),
}, out)
print("written", out, "generated", generated[:, 1:].tolist())
