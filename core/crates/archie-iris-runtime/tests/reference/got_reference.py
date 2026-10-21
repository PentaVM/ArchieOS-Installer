"""Reference outputs of GOT-OCR 2.0 (transformers' GotOcr2ForConditionalGeneration) for the
Rust parity test `tests/ocr.rs`. Test tooling only; nothing at run time uses it.

    python got_reference.py <GOT-OCR-2.0-hf dir> <out dir>

Writes `text.png` (Spanish text rendered with DejaVu Sans, so the test needs no fixture) and
`got-reference.safetensors`: the processor's pixel values and prompt ids, the image tokens the
projector gives, the logits after the prompt, and the greedy reading. CPU, float32.
"""
import sys
from pathlib import Path

import torch
from PIL import Image, ImageDraw, ImageFont
from safetensors.torch import save_file
from transformers import AutoProcessor, GotOcr2ForConditionalGeneration

model_dir, out = Path(sys.argv[1]), Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
torch.manual_seed(0)
torch.set_num_threads(8)

image = Image.new("RGB", (960, 320), "white")
draw = ImageDraw.Draw(image)
font = ImageFont.truetype("/usr/share/fonts/TTF/DejaVuSans.ttf", 56)
draw.text((40, 60), "ArchieOS ve la pantalla", fill="black", font=font)
draw.text((40, 180), "Reunión mañana 18:30", fill="black", font=font)
image.save(out / "text.png")

processor = AutoProcessor.from_pretrained(model_dir)
model = GotOcr2ForConditionalGeneration.from_pretrained(model_dir, torch_dtype=torch.float32).eval()
inputs = processor(image, return_tensors="pt")
with torch.no_grad():
    features = model.model.get_image_features(pixel_values=inputs["pixel_values"]).pooler_output
    logits = model(**inputs).logits[0, -1]
    generated = model.generate(**inputs, do_sample=False, max_new_tokens=32,
                               tokenizer=processor.tokenizer, stop_strings="<|im_end|>")
new = generated[0, inputs["input_ids"].shape[1]:]
text = processor.decode(new, skip_special_tokens=True)
print("read:", repr(text))
save_file({
    "pixel_values": inputs["pixel_values"].contiguous(),
    "input_ids": inputs["input_ids"].to(torch.int64).contiguous(),
    "image_features": features.contiguous(),
    "prompt_logits": logits.contiguous(),
    "generated": new.to(torch.int64).contiguous(),
}, out / "got-reference.safetensors")
(out / "got-reference.txt").write_text(text, encoding="utf-8")
