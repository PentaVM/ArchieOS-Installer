# ArchieOS WSL architecture

ArchieOS Workstation is an Arch Linux WSL2 distribution. `archied` remains the trusted
control plane. Cerebra decides, policy issues bounded leases, and executors act only
inside an authorised workspace. Model APIs and CLI harnesses are replaceable workers.

The runtime tree is `/archie`. Work is confined to `/archie/workspaces/<id>`, model
weights live under `/archie/models`, transient process state under `/archie/runtime`,
and encrypted state under `/archie/state`. `/vault` is owned by root and the `archie`
group and is never exposed as a worker workspace.

Windows drives are not automounted and Windows process interop is disabled in
`/etc/wsl.conf`. Host integration must use the typed bridge contract in
`archie-workstation::host`; its path validator rejects absolute paths and traversal.

`archie-workstation.target` groups six units. The core is the only long-running unit.
The Cerebra, Vault, router, speech and tool units are boot-time contract and dependency
checks, avoiding artificial microservices.

