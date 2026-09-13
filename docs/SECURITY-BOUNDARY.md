# Workstation security boundary

The authority chain is:

```text
intent -> Cerebra -> context refs -> ContextCapsule -> model proposal
       -> policy decision -> expiring capability lease -> executor
```

Models and harnesses cannot grant leases. The FreeCAD adapter accepts an
`AuthorizedWorkspace` that can only be constructed after an `archie-policy::Lease`
authorises a write against the exact workspace resource. It invokes a fixed executable
without a shell and canonicalises scripts below the workspace before execution.

`PrivacyGrade::Restricted` maps to local-only context. `Sensitive` may reach only a
controlled fabric. `Public` and `Internal` may reach cloud only when the model descriptor
and verified retention policy permit it. `Secret` never reaches any model. Cloud endpoint
configuration requires HTTPS and an environment variable name; the secret value is read
only at request time.

The WSL image disables Windows drive automount and interop. Host imports, exports,
notifications, microphone, clipboard and app launch are typed capabilities. The alpha
contains the contract and validation but no privileged Windows host service.

