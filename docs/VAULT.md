# Vault and remote exposure

The existing `archie-vault` crate is the workstation Vault. Entries are encrypted with
AES-256-GCM using per-entry nonces and authenticated metadata. Callers hold opaque
`VaultRef` values. The gateway alone opens entries and emits task-bound, expiring
`ContextCapsule` values; capsules deliberately do not implement `Serialize`.

Credentials use `PrivacyGrade::Secret`. They cannot enter a capsule or reach any model.
Provider configs persist only the environment variable name. At request time
`archie-model-runtime` reads the value and applies it as transport authentication; the
value is absent from prompts, persisted config, learned checkpoints and structured
traces. The WSL image contains no `.env` file or API key.

Workstation exposure levels map to the existing kernel grades:

| Workstation policy | Kernel grade | Maximum locality |
|---|---|---|
| `REMOTE_ALLOWED` | `Public` or admitted `Internal` | Cloud, subject to verified retention |
| `SANITIZED_REMOTE` | capsule downgraded after gateway redaction | Cloud only after the gateway issues the sanitized capsule |
| `LOCAL_ONLY` | `Restricted` | Local |

`Sensitive` is fabric-only. `Secret` reaches no model at any locality. A missing local
provider never causes restricted data to fall back to cloud.

On WSL, `/vault` is root-owned with group traversal only. Workers receive capsule
contents through the gateway and never filesystem access to `/vault`.

Configure a provider credential for the current session, or through a local secret
manager that injects its environment at service start:

```bash
systemctl edit archie-router.service
# Add an EnvironmentFile that is root-readable and outside /archie/workspaces.
```

Never place credentials in `providers.toml`; its schema accepts only an environment
variable name.

