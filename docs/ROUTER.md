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

