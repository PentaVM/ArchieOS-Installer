# Workstation router

The existing `archie-router` ranks model descriptors by capability, locality, privacy,
context capacity, health, cost and utility. `archie-model-runtime` enforces the selected
descriptor again at the network boundary.

Workstation provider config adds discovery records for Nebius Token Factory, Cerebras,
OpenRouter, Ollama and vLLM. Models are discovered from the provider or named in local
config; no model identifier is compiled into the router. Any OpenAI-compatible endpoint
can be added through the same schema. Anthropic is represented as a separate protocol so
it cannot accidentally be sent OpenAI wire messages; its native transport is not present
in this alpha.

Cloud endpoints require HTTPS and the name of an environment-backed credential. Local
HTTP is accepted only for loopback. `Restricted` routes only locally, `Sensitive` only
locally or within the controlled fabric, and `Secret` reaches no model. Failure to find a
local backend is a refusal, never a cloud fallback.

```bash
archie provider add nebius
archie provider add cerebras
archie provider add ollama
archie provider list
archie provider test nebius
```

`provider test` validates configuration without disclosing context. Runtime model
discovery uses the provider's `/models` endpoint.


## Choosing the reasoning model

IRIS is Archie's own eyes and hands and is always there. What thinks is chosen: the
router picks, for each request, among the model manifests in `/etc/archie/models`
(`models/` in this repository), by role, privacy and score.

### A cloud model, with a key

The manifests in `models/nebius/` describe the reasoning models on Nebius Token Factory:
Nemotron, Qwen, GLM, MiniMax, Kimi, DeepSeek, gpt-oss and Hermes. Their context windows
and prices are Nebius's own list; the latency in each is one measured plan. They work
when the key is in the environment of whatever asks:

```bash
# /etc/archie/model.env, mode 0600
NEBIUS_TOKEN_FACTORY_KEY=...
```

The default is `nebius.nemotron-3-super` for an answer and `nebius.nemotron-3-ultra` for
reasoning. To prefer another, raise its `quality_tier`, or remove the manifests you do
not want. Another provider is a manifest with `provider = "openai-compatible"`, a
`base-url` label and a `credential-env` label naming the variable its key is in.

Cloud models are cleared for public data only until an operator has verified the
provider's retention for their account and says so in the manifest.

### A model on the machine

Private requests never go to a cloud model, so a node that should answer them needs a
reasoning model of its own. Run any OpenAI-compatible server on loopback and describe it:

```toml
# /etc/archie/models/local/qwen.toml
id = "local.qwen"
display_name = "Qwen, on this machine"
provider = "openai-compatible"
locality = "local"
context_window = 32768
max_output_tokens = 4096
capabilities = ["cognition.response", "cognition.reasoning"]
max_privacy_grade = "restricted"
availability = "ready"
quality_tier = 60

[cost]
input_micros_per_mtok = 0
output_micros_per_mtok = 0

[labels]
base-url = "http://127.0.0.1:11434/v1/"
provider-model = "the-name-your-server-lists"
```

`provider-model` is the name the server knows the model by. With both kinds installed,
public requests go to whichever scores best and private ones only to the local model.

### Private text and a cloud model

A node with only cloud models can still work on private text, because what makes it
private is veiled before it leaves (`archie_vault::Veil`):

| What | Leaves as |
|---|---|
| API keys, tokens, private keys, anything after `password=`, `token:`, `Bearer` | `SECRET_n` |
| E-mail addresses | `EMAIL_n` |
| Telephone numbers (`+56 9 …`, or a nine-digit Chilean mobile) | `PHONE_n` |
| Payment card numbers that pass the Luhn check | `CARD_n` |
| Chilean RUTs with a valid check digit | `RUT_n` |
| People, companies, projects and accounts the operator named | `PERSON_n`, `COMPANY_n`, … |

The table that reverses the stand-ins stays in the asking process, and the real values
are put back into the answer there. The same value is the same stand-in throughout a
request, so the model can still tell who is who.

```bash
echo 'Write a .env with OPENAI_API_KEY=sk-… and alerts to camila@acme.cl' \
  | archie-ask --grade sensitive --know person="Camila Rojas"
```

`archie-ask` prints the answer, and on standard error which model answered and how many
values of each kind were kept on the machine.

What decides where a private question goes:

1. A model on the operator's own machines that may answer is asked, unveiled. Nothing
   leaves.
2. Otherwise, text graded up to `sensitive` is veiled and sent to the cloud model the
   router chooses. Text nobody graded is veiled too: a key pasted into an ordinary
   question is still a key.
3. `restricted` text, `secret` text and pictures are not sent. The veil removes what
   identifies and unlocks, not what a text means, and pixels cannot be veiled.
4. If a second look at the veiled text still finds something with a sensitive shape,
   nothing is sent.

Detection is by shape and by the names the operator registered. A value with neither (a
street address, a nickname) is not found; register it with `--know`, or keep the request
`restricted`.

### From code

`archie_model_runtime::routed::Routed` asks whichever model the router chooses for a
role and tries the next when one fails; `ask_private` adds the rules above. The live
Arena's planner uses the first.
