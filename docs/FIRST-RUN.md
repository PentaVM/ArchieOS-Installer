# First run

```bash
archie setup
archie doctor
sudo /usr/lib/archie/install-nemotron-speech
archie speech model status
archie tools test freecad
```

`archie doctor` checks WSL2, systemd, workspace and Vault directories, NVIDIA GPU
visibility, FreeCAD, local speech and router contracts. Missing optional hardware is a
warning; missing filesystem or service boundaries is a failure.

Configure one inference backend. The model name is discovered rather than fixed:

```bash
archie provider add nebius     # then inject NEBIUS_TOKEN_FACTORY_KEY locally
# or
archie provider add ollama     # local loopback; no API key
```

Harnesses are optional. Install each upstream CLI under its own terms, then run
`archie harness login <id>` and `archie harness test <id>`. Credentials remain in the
upstream CLI's own store.

Windows drives are intentionally unavailable inside the runtime. Files enter or leave a
workspace through the host bridge contract; the privileged Windows bridge itself remains
an explicit alpha limitation.

