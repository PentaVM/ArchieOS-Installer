# First run

```bash
archie setup
archie doctor
sudo /usr/lib/archie/install-nemotron-speech
scripts/models/install-piper-runtime.sh
scripts/models/install-piper-voices.sh
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

The Piper pack installs separate Spanish and English voices under the ignored local
directory `models-cache/piper`. Archie selects one for each answer sentence, based on the
language of the generated text. The ONNX files are checksum-verified and stay outside Git;
their model cards and recording dataset terms are installed alongside them.
The runtime script installs the pinned Python Piper and ONNX Runtime packages into the
active Python environment. Run it inside the environment that will launch `run-archie.sh`.

Harnesses are optional. Install each upstream CLI under its own terms, then run
`archie harness login <id>` and `archie harness test <id>`. Credentials remain in the
upstream CLI's own store.

Windows drives are intentionally unavailable inside the runtime. Files enter or leave a
workspace through the host bridge contract; the privileged Windows bridge itself remains
an explicit alpha limitation.

The separate [Operator image](WSL-OPERATOR.md) has its own WSL configuration and
currently omits voice. Its committed config enables automount and disables interop;
the isolated runtime description above does not describe that image. Importing its
scripts into main does not establish a tested host bridge or a speech installation.
