# Data provider manifests

One TOML file per upstream data source. These are **data** providers, the feeds Sentry
observes the world through. They are separate from *model* providers (inference
backends), which are a different concept with a different registry.

Adding a feed means writing one of these. A feed that cannot be described here is a feed
Archie will not fetch, so that an operator can see, enable and disable every place their
node reaches out to.

**No manifest ever contains a credential.** A provider that authenticates names the
secret it needs (`secret_handle`); the value is resolved at fetch time from the node
secret store. `ProviderManifest::validate` rejects a manifest that authenticates without
naming a secret, and one that names a secret it does not need.

`protocols/provider.schema.json` describes the same data model for the Python workers and
external tooling. TOML here is for operators; the schema is the language-independent
contract.

## Status on a given node

A manifest existing does not mean it runs. `ProviderRegistry::resolve` decides, and
reports a reason for everything excluded:

| Status | Meaning |
|---|---|
| `available` | will poll |
| `no-capability` | the node does not advertise the capability the provider satisfies |
| `not-authorized` | the capability family needs an explicit operator grant |
| `no-credentials` | the named secret is not configured, which is ordinary, not an error |
| `disabled` | turned off by configuration |
| `invalid` | the manifest itself is wrong |

## Known gaps are not optional

Every manifest here records what its feed CANNOT see. That field is what stops an empty
region being read as a quiet one: ADS-B is sparse over oceans, FIRMS cannot see through
cloud, and a conflict database only covers the countries its coders staff. A layer
rendered without its gaps lies by omission.
