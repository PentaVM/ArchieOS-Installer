# NVIDIA and Nebius model providers

ArchieOS uses the same OpenAI-compatible transport for NVIDIA NIM, Nebius Token
Factory, and future compatible runtimes. Model identity, cognitive role, provider, and
locality remain separate router properties.

## Local NVIDIA NIM

Start a compatible NIM container according to that model's NVIDIA support matrix, then:

```bash
export ARCHIE_NVIDIA_NIM_BASE_URL=http://127.0.0.1:8000/v1
printf 'Return one sentence describing the current node.\n' | \
  cargo run --manifest-path core/Cargo.toml -p archie-model-runtime --bin archie-infer -- \
  --provider nvidia-nim --locality local \
  --base-url "$ARCHIE_NVIDIA_NIM_BASE_URL" --model '<served-model-id>'
```

Local loopback NIM does not require an API key at this boundary. Container registry
authentication and model licenses remain NVIDIA concerns outside Archie configuration.

## Nebius Token Factory

```bash
export NEBIUS_TOKEN_FACTORY_KEY='<secret>'
export ARCHIE_NEBIUS_BASE_URL=https://api.tokenfactory.nebius.com/v1
cargo run --manifest-path core/Cargo.toml -p archie-model-runtime --bin archie-infer -- \
  --provider nebius --base-url "$ARCHIE_NEBIUS_BASE_URL" --list-models
```

Cloud requests are limited to `public` or `internal` privacy grades. `sensitive` and
`restricted` input is rejected before any HTTP request is created. Remote endpoints
require HTTPS, URLs cannot contain credentials, and response bodies have a byte budget.

No model ID is hardcoded. Use the provider's model discovery endpoint and register the
chosen model with the Archie router according to its actual capabilities and limits.
