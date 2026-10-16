"""Native runtime application proof for synthetic, unpromoted one-layer adapters."""
import json
from pathlib import Path
import urllib.request

key = (Path.home() / '.local/state/archie-bonsai-compat/api.key').read_text().strip()
def request(route, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    message = urllib.request.Request('http://127.0.0.1:18081/' + route, data=data,
        headers={'Authorization': 'Bearer ' + key, 'Content-Type': 'application/json'})
    with urllib.request.urlopen(message, timeout=15) as response:
        return json.load(response)

loaded = request('lora-adapters')
print('Native adapters:', json.dumps(loaded))
def configure(zero, nonzero):
    return request('lora-adapters', [{'id': 0, 'scale': zero}, {'id': 1, 'scale': nonzero}])
def complete():
    return request('v1/chat/completions', {'messages': [{'role': 'user', 'content': 'Responde solamente con la palabra LISTO.'}],
        'temperature': 0, 'max_tokens': 1, 'logprobs': True, 'top_logprobs': 5,
        'chat_template_kwargs': {'enable_thinking': False}})
try:
    configure(0, 0)
    baseline = complete()
    configure(1, 0)
    zero = complete()
    configure(0, 1)
    nonzero = complete()
    configure(0, 0)
    restored = complete()
finally:
    configure(0, 0)

def scores(response):
    return {item['token']: item['logprob'] for item in response['choices'][0]['logprobs']['content'][0]['top_logprobs']}
base_scores = scores(baseline)
def delta(response):
    other = scores(response)
    return max(abs(value - other[token]) for token, value in base_scores.items() if token in other)
report = {'scope': 'synthetic_one_layer_adapter_loading_only', 'full_qlora_training': False,
          'promoted': False, 'native_adapters': loaded, 'zero_delta': delta(zero),
          'nonzero_delta': delta(nonzero), 'restore_delta': delta(restored),
          'responses': {'base': baseline, 'zero': zero, 'nonzero': nonzero, 'restored': restored}}
output = Path('/mnt/c/Users/Penta/Documents/ChatGPT/ArchieOS/_dev/verification/rtx3070/results/2026-10-16-bonsai-native-lora.json')
assert not output.exists()
output.write_text(json.dumps(report, indent=2))
print(json.dumps({name: value for name, value in report.items() if name != 'responses'}, indent=2))
assert report['zero_delta'] < 1e-5 and report['restore_delta'] < 1e-5 and report['nonzero_delta'] > 1e-7
