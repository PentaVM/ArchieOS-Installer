# ArchieOS 0.1 live node

The live image starts `archied` automatically as an unprivileged service. It also enables
the socket for `archie-cortexd`, an unprivileged, on-demand deterministic context compiler.
It remains idle until the trusted knowledge layer provides a graph snapshot and `archied`
requests a bundle; it is not a model runtime or a source of authority.

Useful commands:

```text
archie ping
archie status
archie topology
archie peers
journalctl -u archied
systemctl status archied
systemctl status archie-cortex.socket
journalctl -u archie-cortex
archinstall
```

The live profile is intentionally `dev-vm`: it must not become the sole canonical copy
of knowledge. Choose the installed node role explicitly during installation/first boot.
