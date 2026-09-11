# installer

**Status: WRITTEN, NOT YET RUN ON HARDWARE.** Every decision is implemented and tested;
the wizard around them is implemented and has never partitioned a real disk, because
that needs the ISO and the ISO has not been built.

`archie-install`, run from the live ISO to put ArchieOS on disk.

## The shape

An installer is mostly prompting, and almost nothing it gets wrong is in the prompting.
It is in the decisions. So the decisions are in `core/crates/archie-installer` and run on
a machine with no disk in it, and `src/main.rs` is the asking and the doing.

```
archie_installer::disk     which disk may be erased, and why the others may not
archie_installer::probe    what `lsblk` said, understood
archie_installer::answers  names, checked against what the kernel and useradd accept
archie_installer::config   the archie.toml the install writes
archie_installer::plan     every step, as data, before any of it happens
archie_fabric::enrolment_plan   which fabric the machine ends up in
```

35 tests. Fifteen mutations of the decisions below; all fifteen killed.

## The decisions

**The medium you booted from is never offered.** A refused disk is shown with its reason
rather than filtered out of the list. Somebody who cannot find the disk they expected
concludes the installer is broken and goes looking for the flag that turns the protection
off; somebody told *this is the stick you are booting from* stops looking.

**`/dev/nvme0n1` + 1 is `/dev/nvme0n1p1`.** An installer that assumes `/dev/sda1` form
writes a filesystem to a path that does not exist, or on an unlucky machine to one that
does. The `p` goes in when the device name ends in a digit, which is the rule the kernel
follows and the reason `nvme0n1p11` is not ambiguous.

**The live medium is found one level down.** It announces itself through a mount point on
a partition, so the parser recurses — and a disk whose crypt volume is mounted is as much
in use as one whose first partition is.

**Confirmation is typing the disk path.** Not "yes". "Yes" is what somebody types without
reading, and the sentence they did not read is the one naming the disk.

**No step of the plan can hold a secret.** A plan is printed, and on a failure it is the
first thing pasted into a bug report. The password goes to `chpasswd` down a pipe, never
an argument and never a file; the private keys are referred to by path, staged in a tmpfs
that does not survive a power cycle.

**The generated `archie.toml` is parsed by the real loader before it is written.** A node
that will not boot because its own installer produced configuration the daemon rejects
should fail here, in a second, not on the next power cycle.

## Which fabric

`FabricChoice` has two arms and no guest flag:

| Choice | What happens |
|---|---|
| `StartNew` *(default)* | A fresh owner key is generated. This machine owns its own fabric. |
| `JoinWithOwnerKey` | The machine is admitted to an existing fabric, and does **not** keep the owner key. |

There is deliberately no "guest mode" switch. A switch is a thing that can be left in the
wrong position, and the person it would harm is the one least likely to check it. Every
install starts its own fabric unless somebody supplies an owner key they could only have
got off a machine they control — so isolation is what happens when nobody does anything
special, and joining is the step that takes an action.

Two people who each install ArchieOS on the same network end up with two fabrics. Neither
answered a question about it, and neither can reach the other. See
`core/crates/archie-fabric/tests/enrolment_plan.rs` and `isolation.rs`.

The private owner key admits nodes. It is not needed to *run* one, so it can live offline
— on a stick in a drawer — and a fabric whose owner key is nowhere reachable keeps
working; it simply cannot admit anything new until the key comes back. A machine that
joins an existing fabric is never given a copy: it was lent the key long enough to issue
one certificate for itself.

## Afterwards

`archie enrol` makes the same decision through the same code, for a machine that was
installed and not enrolled. That is a real state, not a broken one: it boots, it runs, it
talks to nobody, and `archied` reports it as a degraded boot stage with the reason, so an
operator does not read an empty peer list as a network fault.

## What has not been proved

The wizard has never run. The library half is tested and mutation-tested; the half that
partitions a disk, chroots and installs a bootloader has been read and not executed, and
until an ISO boots on real hardware that is exactly as much as anyone should assume.
