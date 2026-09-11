# RFC-007 — Kernel-level integration

- **Status:** partially implemented. §3.7, §3.1, §3.9, §3.2 and §3.3 have landed as
  parsers and decision logic with fixture-backed tests; see §5 for the live state of each
  item. §7 is **accepted and in progress**: the execution layer is being built, and it
  gates every remaining worker-specific item. §3.4 onward wait on it.
- **Scope:** what ArchieOS should get from the Linux kernel directly, rather than from
  userspace tools installed alongside it.

## 1. The distinction that matters

ArchieOS is currently a very well-behaved *application* that happens to boot on Arch. It
asks politely for almost everything:

| Today | What it actually is |
|---|---|
| the display "accepts a quality budget" | a request the renderer may ignore |
| idle work "checkpoints before yielding" | cooperative; a wedged worker never yields |
| a capability lease authorises a file read | checked by `archie-policy`, then the worker opens the file anyway |
| hardware is probed at boot | a poll; a GPU plugged in at 14:00 is invisible until reboot |
| the node identity is a 0600 file | copy the file, become the node, forever |
| "every particle is a real process" | `/proc` polling at 1 Hz |

Every row is a place where Archie *describes* a rule and something else could break it.
The kernel is the only component on the machine that can make a rule true rather than
advisory. That is the whole content of this RFC: move the load-bearing guarantees from
"Archie asks" to "the kernel refuses".

This is explicitly **not** a list of tools to install. Each item below is a kernel
interface — a filesystem under `/sys` or `/proc`, a syscall, an LSM, a netlink socket —
that `archied` talks to directly.

## 2. What is already kernel-enforced

Credit where it is due; this is a real foundation, not nothing.

- **cgroup v2 limits on the observatory.** `archie-display.service` carries
  `CPUWeight=20`, `CPUQuota=100%`, `MemoryHigh=768M`, `MemoryMax=1536M`, `TasksMax=128`,
  `OOMPolicy=stop`. systemd writes these into the unit's cgroup; the kernel enforces
  them. A runaway renderer is throttled and then killed without archied noticing.
- **Sandboxing on both daemons.** `ProtectSystem=strict`, `CapabilityBoundingSet=`,
  `NoNewPrivileges`, `RestrictAddressFamilies`, `MemoryDenyWriteExecute`,
  `SystemCallArchitectures=native`, `IPAddressDeny=any`. These are seccomp filters, LSM
  hooks and mount namespaces — kernel mechanisms, configured declaratively.
- **Account separation.** `archie` and `archie-display` are distinct uids; the control
  socket is group-gated at 0660. That is kernel access control, not application logic.
- **tmpfiles/sysusers.** Ownership and modes applied by PID 1 before anything starts.

So the question is not "should ArchieOS use the kernel" — it already does. The question
is which of the *remaining* advisory rules should become enforced ones.

## 3. The integrations, in order of value

### 3.1 PSI — the correct idle signal

**`/proc/pressure/{cpu,memory,io}`**

The scheduler's whole idle-compute design rests on knowing when the machine is idle, and
it currently has no good way to tell. CPU percentage is a bad proxy in both directions: a
batch job pinning 8 cores without stalling anything is *fine*, and a machine at 20% CPU
that is thrashing memory is *not*. Pressure Stall Information measures the thing that
actually matters — how much time runnable work spent waiting on a resource.

```
some avg10=0.00 avg60=0.00 avg300=0.00 total=0
full avg10=0.00 avg60=0.00 avg300=0.00 total=0
```

`some` is "at least one task stalled"; `full` is "everything stalled". A deep-idle
research worker should key off `cpu.some avg10 < 1%` and `io.full avg10 ≈ 0`, and a user
request arriving should be visible as pressure within ten seconds.

- **Replaces:** the `NodeLoad.cpu_utilization` heuristic in the scheduler.
- **Effort:** small. Three files, a parser, a sampler.
- **Testable off-Linux:** yes — the format is stable text; parse against fixtures.

### 3.2 cgroup v2 written directly, not just declared

**`/sys/fs/cgroup/**/{cpu.max,memory.high,io.weight,cgroup.freeze}`**

systemd sets a *static* envelope at unit start. The resource governor in RFC-006 needs a
*dynamic* one: under inference pressure the display's budget should fall, and on an idle
machine it should rise. That means archied writing `cpu.max` to the display's cgroup at
runtime, which systemd supports through its delegation model.

More importantly, `cgroup.freeze` changes what preemption means. Today the scheduler
emits `Preempt { must_checkpoint: true }` and *hopes* the worker checkpoints and exits. A
worker stuck in a tight loop never does, and the user request waits. With the freezer:

```
1. ask the worker to checkpoint            (cooperative, bounded by a deadline)
2. on deadline, write 1 to cgroup.freeze   (kernel stops it mid-instruction)
3. start the user's work
4. thaw or kill later
```

The cooperative path stays first because a checkpoint is worth waiting a second for. The
freezer is what makes the deadline mean something.

- **Replaces:** advisory preemption; the display's self-policed budget.
- **Effort:** medium. Requires `Delegate=yes` on the units and careful path discovery.
- **Risk:** freezing a process holding a lock the control plane needs. Only ever freeze
  leaf worker cgroups, never anything archied talks to synchronously.

### 3.3 udev + netlink — hardware as events, not polls

**`AF_NETLINK`/`NETLINK_KOBJECT_UEVENT`, and `/sys` for enumeration**

"Adding hardware must expand Archie without architectural redesign" is a founding claim,
and today it is only true across a reboot. Capability probing runs once, at boot.

Subscribing to the uevent netlink socket makes it literally true: a GPU, a CAN adapter, a
LoRa radio, an ESP32 on USB, a disk — each arrival is a kernel event that can add a
capability to the live manifest and trigger a fabric re-announce. Removal likewise, which
is the case that matters more: a GPU that fell off the bus should stop being schedulable
*now*, not at the next boot.

The boot-to-boot capability diff already in `archie-state` becomes the coarse view, and
this becomes the live one.

- **Replaces:** `HardwareProbe::run()` as the only source of truth.
- **Effort:** medium. The socket is easy; the classification rules are the work.
- **Testable off-Linux:** the event *parser* yes, the socket no.

### 3.4 Landlock — make a capability lease something the kernel enforces

**`landlock_create_ruleset(2)`, kernel ≥ 5.13, unprivileged**

This is the largest gap between what the architecture claims and what is true.
`archie-policy` issues a lease scoped to `/var/lib/archie/knowledge`, and a worker that
ignores the lease and opens `/etc/shadow` succeeds, because nothing enforced anything.

Landlock is an LSM a process can apply *to itself*, without privilege, and cannot escape.
The sequence is exactly the shape of a lease:

```
worker starts
→ archied grants a lease naming paths
→ worker applies a Landlock ruleset matching the lease, then drops the ability to widen it
→ the kernel now refuses every open() outside the grant
```

The lease stops being a claim about what a worker *should* do and becomes a fact about
what it *can* do. Paired with `seccomp` for syscall classes, and with the existing
`RestrictAddressFamilies`, a Python research worker becomes genuinely contained.

- **Replaces:** capability leases as advisory bookkeeping.
- **Effort:** medium. Needs a small syscall wrapper; the `landlock` crate exists.
- **Note:** ABI versions differ across kernels; the code must negotiate best-effort and
  report the version it got, because silently getting a weaker sandbox is the failure
  mode.

### 3.5 TPM 2.0 — an identity that cannot be copied

**`/dev/tpmrm0`, sealing against PCRs**

`NodeId` is the SHA-256 of an Ed25519 public key, and the private key is a 0600 file.
That is honest and it is also copyable: anyone with root, or with the disk, becomes that
node permanently and the fabric cannot tell.

Sealing the key to the TPM changes the threat model. The key is generated in, or
encrypted to, the TPM and unsealed only when the PCRs match — i.e. only on this machine,
booted with this measured configuration. Copying `/var/lib/archie/identity/node.key`
yields nothing.

This is what makes the "trust state" model on the receiving side worth anything: pinning
a peer's key means something only if that peer cannot have its key stolen.

- **Replaces:** a file-based private key.
- **Effort:** medium-high. Needs a fallback: not every node has a TPM, and a node without
  one must still work while *reporting* that its identity is software-held. A fabric that
  silently mixes hardware-backed and file-backed identities is worse than one that says
  which is which.

### 3.6 eBPF — make "every particle is a real process" true

**`aya` (pure-Rust, no libbpf/clang toolchain)**

The display's stated contract is that every visual element encodes something real. The
process constellation is currently `/proc` polling, which gives a snapshot and no flows:
it cannot show that archied talked to the fabric, or that a worker opened a socket, or
where bytes went.

Attach points worth having, and only these:

| Hook | Gives |
|---|---|
| `sched_process_exec` / `exit` | real process lifecycle, not a 1 Hz diff |
| `tcp_connect` / `inet_sock_set_state` | actual edges between processes and peers |
| `sock_sendmsg` byte counters | edge weight that means bytes/sec |
| `oom_kill_process` | why something vanished |

The brief's own warning applies: do not trace everything blindly. Four probes with
per-cgroup filtering, not a firehose.

- **Replaces:** polled `/proc` topology; makes edges real rather than inferred.
- **Effort:** high. Needs CAP_BPF, a verifier-friendly program, and a careful story about
  what happens on kernels without BTF.
- **Honest note:** this is the item most likely to be *deferred*. It is the most
  impressive and the least load-bearing; §3.1–3.4 are worth more.

### 3.7 systemd readiness and watchdog

**`sd_notify(3)`, `WatchdogSec=`**

`archied` is `Type=simple` today, so systemd considers it started the moment it is
`exec`'d — before identity, state, fabric or topology exist. Anything ordered `After=`
archied races it.

`Type=notify` plus `sd_notify("READY=1")` at the exact point the boot report reaches
ARCHIE READY makes readiness a real signal that PID 1 knows about, and makes
`archie-displayd` orderable correctly. `WatchdogSec=30` with a periodic
`sd_notify("WATCHDOG=1")` turns a hung control plane into a restart instead of a silent
stall — and the ping should be emitted from the main loop, not a timer thread, or it
proves nothing.

- **Effort:** small. This is the highest value-per-line item in the whole RFC.

### 3.8 Btrfs snapshots from archied, not from a script

**`BTRFS_IOC_SNAP_CREATE_V2`**

Btrfs with pre-change snapshots is already the stated rollback story, and it is currently
shell. Doing it through the ioctl inside archied means a snapshot can be taken *as part
of* a transition — before a model promotion, before applying a patch Forge built, before
`pacman -Syu` — and recorded in the audit trail with the same id the state store uses. A
rollback then has a claim to point at.

### 3.9 Thermal, power, and the hardware watchdog

**`/sys/class/hwmon`, `/sys/class/thermal`, `/sys/class/powercap`, `/dev/watchdog`**

Thermal telemetry belongs in the capability manifest and in the safety story: a node
above its throttle point is `Degraded` and should stop taking new inference work before
the kernel throttles it for us.

`/dev/watchdog` is the harder-edged one. On a node driving anything physical, a hardware
watchdog that reboots the box when userspace stops petting it is the difference between a
hung machine and a hung machine *that stopped commanding actuators*.

### 3.10 Real-time scheduling — with a clear limit

**`SCHED_DEADLINE`, `PREEMPT_RT`**

`SCHED_DEADLINE` and an RT kernel narrow the latency window for a control path, and are
worth having on a robot/suit gateway.

They do **not** make Linux hard real-time, and this RFC will not pretend otherwise. The
architecture's rule stands unchanged: the deterministic controller is an MCU, and the
E-stop is hardware wired independently of anything in this document. Kernel scheduling
improves the soft path; it does not move the safety boundary.

## 4. What this does not include, and why

- **A custom kernel module.** Everything above uses stable kernel ABIs. An out-of-tree
  module would need rebuilding for every kernel update, would break `linux-lts` vs
  `linux`, and buys nothing that eBPF and the existing interfaces do not.
- **Patching the kernel.** Same reasoning, worse.
- **Replacing systemd.** systemd *is* the supervision and sandboxing layer; the units
  already carry the LSM and cgroup configuration. Talking to cgroups directly (§3.2) is
  done through systemd's delegation, not around it.
- **Putting the GUI inside the kernel.** Rejected: drivers, authority and enforcement
  belong in kernel space; scene graphs, fonts, textures and provider parsing do not. A
  UI fault must not panic the node.
- **Direct KMS/DRM from a native system client.** Accepted and implemented. The compiled
  C++/Qt Quick 3D process uses EGLFS/KMS with no browser, Electron, HTML/JavaScript app,
  compositor or desktop, while remaining a killable user-space cgroup outside authority.

## 5. Order of work, and where each item stands

| # | Item | State | Where |
|---|------|-------|-------|
| 1 | §3.7 sd_notify + watchdog | **landed** | `archied/src/notify.rs`, `archied.service` is `Type=notify` |
| 2 | §3.1 PSI | **landed** | `archie-observability/src/kernel/psi.rs`, wired into `plan_under_pressure` |
| 3 | §3.9 thermal | **landed** (trip points; no hardware watchdog yet) | `archie-observability/src/kernel/thermal.rs` |
| 4 | §3.2 cgroup freeze + dynamic budget | **landed, not yet wired** | `archie-observability/src/kernel/cgroup.rs` |
| 5 | §3.3 udev hotplug | **landed, not yet wired** | `archie-observability/src/kernel/udev.rs` |
| 6 | §3.4 Landlock | proposed | — |
| 7 | §3.5 TPM | proposed | — |
| 8 | §3.6 eBPF | proposed. Most visible, least load-bearing | — |

"**landed, not yet wired**" is a deliberate state, not an unfinished one. The parser and
the decision logic land first, with their tests, and change no behaviour; wiring them into
the scheduler and the registry is a separate change whose blast radius can be reviewed on
its own. A bug in a parser is found by a test. A bug in the wiring stops a node from
scheduling, and should not arrive in the same commit as eight hundred lines of parsing.

Items 1–5 are parsers and a handful of syscalls, and are testable on any host against
fixtures. Items 6–8 need a Linux machine to exercise, which means the RTX 3070 box.

### 5.1 What is still cooperative after items 1–5

Worth stating plainly, so the gap is not mistaken for closed:

- The cgroup writer exists; **nothing calls it yet**. Preemption still asks and waits.
- The uevent parser exists; **no netlink socket is opened yet**. Capability detection is
  still once, at start.
- A capability lease is still checked by `archie-policy` and then the worker opens the
  file anyway (§3.4).
- The node identity is still a 0600 file (§3.5).

### 5.2 One lesson from implementing items 1–3

`cfg(unix)` code does not compile on a Windows development host — it is not merely
untested, it is unparsed for types. `SocketAddr::from_abstract_name` was written against
a constructor that does not exist (the abstract namespace is Linux-only, so it lives on
`std::os::linux::net::SocketAddrExt`), and thirteen green tests said nothing about it
because none of them touched that branch. CI caught it.

The cheap fix is a cross-check that needs no C toolchain and no linker:

```bash
rustup target add x86_64-unknown-linux-gnu
rustc --target x86_64-unknown-linux-gnu --emit=metadata --edition 2021 -o out.rmeta <file>
```

Any `cfg(unix)` or `cfg(target_os)` block should go through that before it is pushed. It
reproduces the exact compiler error in seconds. This reinforces §6: the more logic lives
in platform-independent functions, the less of the codebase depends on that check.

## 6. The service sandbox is the binding constraint

Discovered while wiring §3.2 and §3.3, and it changes the shape of the remaining
work: **`archied.service` is hardened well enough that most of this RFC cannot run
inside it as written.** The parsers are correct and tested; the syscalls they exist
to make are refused by the unit before the kernel ever sees them.

`archied` runs as `User=archie` with:

| Directive | Effect | Blocks |
|---|---|---|
| `CapabilityBoundingSet=` (empty) | no capabilities at all, ever | anything needing `CAP_NET_ADMIN`, `CAP_SYS_ADMIN` |
| `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6` | `socket(AF_NETLINK, …)` fails with `EAFNOSUPPORT` | §3.3 udev — the netlink socket cannot be opened |
| `ProtectControlGroups=yes` | `/sys/fs/cgroup` mounted read-only for the unit | §3.2 cgroup — every write returns `EROFS` |
| `ProtectKernelTunables=yes` | `/proc/sys` read-only | nothing here; `/proc/pressure` reads are unaffected |
| `NoNewPrivileges=yes` | no privilege escalation | §3.4 Landlock is *fine* — Landlock is explicitly designed to be usable unprivileged |

This is not an argument for deleting those lines. They are why a compromised
`archied` cannot reconfigure the machine, and that property is worth more than
any single integration in this document. The point is that "kernel-level" work
now has a second question attached to every item: **not just does the kernel
expose it, but is this unit allowed to ask.**

### 6.1 What each blocked item would actually need

Each of these must be verified on the RTX 3070 box before being written into the
unit — the interactions between systemd's sandboxing directives are not always
what the names suggest, and a wrong guess here silently loosens the sandbox.

- **§3.3 udev** — needs `AF_NETLINK` added to `RestrictAddressFamilies`. Binding
  the *kernel* uevent group (group 1) additionally needs `CAP_NET_ADMIN`, which
  the empty bounding set forbids; the **udev monitor group (group 2)** is the
  unprivileged path and is what `libudev` uses for exactly this reason. So the
  monitor format — which `kernel::udev::Uevent::parse_monitor` already handles —
  is not an alternative here, it is the only viable one.
- **§3.2 cgroup** — needs `Delegate=` so the unit owns a writable subtree, rather
  than dropping `ProtectControlGroups=`. Delegation grants write access to the
  unit's *own* cgroup and nothing above it, which is the whole requirement:
  archied must throttle and freeze its workers, never anything else on the
  machine. `CgroupPath::is_freezable` already refuses the control plane by name;
  delegation would make that refusal redundant rather than load-bearing, which is
  the correct direction.
- **§3.5 TPM** — needs access to `/dev/tpmrm0`, so a `DeviceAllow=` entry.
- **§3.6 eBPF** — needs `CAP_BPF` and `CAP_PERFMON`. This is the item that most
  clearly does not belong inside the hardened daemon; a separate, minimal,
  short-lived collector is the honest design.

### 6.2 What this means for the code that has already landed

Nothing landed so far is wasted, and nothing landed so far is lying:

- The parsers (§3.1, §3.9, §3.3) only ever **read** `/proc` and `/sys`, which the
  sandbox permits. They work today.
- The cgroup **writer** compiles, is tested against a fixture hierarchy, and will
  return `EROFS` on a real node until delegation is configured. That is the
  correct behaviour for now — a refused write is reported, not swallowed.
- `Runtime::refresh_capability_class` is correct and has no caller yet, because
  the thing that would call it cannot open its socket.

The remaining work on §3.2 and §3.3 is therefore **not more Rust**. It is a unit
change, verified on real hardware, that grants the narrowest possible additional
access — and that is a change worth making deliberately rather than folding into
a feature commit.

## 7. The prerequisite this RFC assumed and does not have

Checked while planning §3.4, and it outranks everything in §6.

**Nothing executes tasks.** `Scheduler::plan` is called only from the scheduler's
own tests. `register_running` has no caller outside its crate. `plan_under_pressure`,
added in this RFC's §3.1 work, has no caller either. The runtime constructs a
`Scheduler` and reads `running_count()` from it; it never drives it. The only
`Command::new` in the workspace runs `nvidia-smi` during hardware probing.

The scheduler is a well-tested library that decides what should run, and nothing
turns those decisions into processes.

### Status: the finding above has been addressed, except for one piece

That diagnosis was true when it was written and drove everything in this section. What
exists now, all tested:

| Link | Where |
|---|---|
| submit | `TaskExecutionService::submit` (`archie-core/src/task_service.rs`) |
| queue | `archie-scheduler/src/queue.rs` — readiness from the dependency graph, transitive cancellation |
| plan | `run_pass` (`archie-core/src/dispatch.rs`), driven by `TaskExecutionService::tick` |
| dispatch | `run_pass` again: every decision comes back as an outcome the scheduler is told about |
| bridge | `archie-core/src/execution.rs` — a start whose outcome is unknown is never retried |
| executor | `archie-executord` |

`Scheduler::plan` and `register_running` now have callers. The service is tested end to end
against a scripted executor: a ready task starts and is marked running; a blocked task waits
and starts on the same tick its dependency's completion is drained; a failed dependency
cancels its dependent rather than starting it.

**What is still missing is exposure, and one design decision.** `archied` does not yet accept
a submit request or tick the service, and — the real gate — nothing resolves a module's
`[worker]` declaration (`Python { module, entrypoint }` or `Process { command, args }`) into a
`ResolvedExecutable`. Where the interpreter lives, how a module's `permissions` become
filesystem and network grants, what working directory a worker gets: that is the
worker-execution model, and it is a decision rather than glue. Until it is made, the chain is
complete but has nothing to run.

That has consequences for the rest of this document, and they are not small:

| Item | What it needs that does not exist |
|---|---|
| §3.2 `cgroup.freeze` | a worker cgroup. There are no worker processes to put in one. |
| §3.4 Landlock | a process to restrict. Landlock applies to the calling thread and its children irrevocably; with all work in-process there is no per-task boundary it could express. |
| §3.6 eBPF `sched_process_exec` | processes to observe. The particles would be archied and nothing else. |
| Preemption + `must_checkpoint` | something that could receive the signal and checkpoint. |

§3.1, §3.7 and §3.9 are unaffected — they observe the machine, not the work, and
they are correct and running today.

### 7.1 ACCEPTED — the execution layer is required before further worker-specific enforcement

Decided. Everything below §3.9 that acts on *workers* is blocked on this, so it
is now the canonical next milestone rather than an open question.

The architecture keeps `archied` hardened. It does **not** get loosened so it can
manipulate arbitrary processes and cgroups; instead the privilege is moved into a
separate, much smaller process that has only what it needs:

| Component | Language | Role | Privilege |
|---|---|---|---|
| `archied` | Rust | authority — decides *what* should run | unchanged, still `CapabilityBoundingSet=` empty |
| `archie-executord` | Rust | mechanism — materializes a decision as a process | owns one delegated cgroup subtree, nothing above it |
| `archie-task-runner` | Rust | sandbox — applies the irreversible boundary, then `exec`s | drops everything it does not need |
| worker | any | the actual workload | untrusted |

`archied` decides. The executor materializes. The runner confines. Splitting the
last two matters: Landlock is irreversible for a process tree, so the thing
applying it must be the thing about to become the worker, and must not be a
long-lived supervisor that still has other tasks to start.

The C++-only rule from RFC-008 applies to Sentry's analytics and display stack.
This is trusted control-plane infrastructure and stays Rust.

### 7.2 Order of work

1. execution contracts + durable lifecycle ← **landed**
2. `archie-executord`
3. real `Start` / `Stop` / `Preempt` / `Resume` wiring
4. delegated per-task cgroups — where §3.2's writer stops being inert
5. checkpoint protocol
6. kernel freeze fallback
7. Landlock on the task runner (§3.4), never on `archied`
8. crash and restart reconciliation
9. live udev hotplug (§3.3), through the unprivileged monitor group
10. TPM-backed identity (§3.5)
11. eBPF (§3.6), last — it is only useful once worker processes exist to observe

### 7.3 What the contracts already refuse

`archie-execution` landed first, and it is entirely I/O-free: no spawning, no
cgroup writes, no sockets. Two rules carry most of the weight.

**There is nowhere to put a shell command.** `TaskExecutionSpec` has no field
holding a command line, and `ResolvedExecutable`'s fields are private with a
single constructor the registry calls. A path cannot be supplied by a caller,
typed by an operator, or invented by a model. Relative programs are refused
because resolving one would make what actually runs a function of the daemon's
`PATH` rather than of the manifest.

**A requirement the node cannot meet is a placement failure, not a downgrade.**
If a spec demands filesystem sandboxing and this kernel's Landlock is too old,
the task is refused here — with the shortfall named, so the scheduler can place
it elsewhere. Running untrusted work with less isolation than its manifest
demanded, quietly, is the worst outcome available.

`SandboxStrength` is what a node reports about itself, and it reports honestly.
A development host and a misconfigured node produce the same all-false answer on
purpose, so neither is mistaken for a working sandbox.

And the lifecycle keeps `Checkpointed` and `Frozen` apart. Both mean the task
stopped. Only one means the work survives being killed — a frozen task's state
lives in the memory of a process that is one `SIGKILL` from gone, and recording
both as "paused" would make a week of idle research look recoverable when it is
not.

## 8. Testability note

Every `/proc` and `/sys` interface above is stable text with a documented format. The
parsers can be written and tested anywhere against captured fixtures; only the *effects*
need Linux. That split is worth preserving deliberately: put the parsing and the decision
logic in testable pure functions, and keep the syscall surface as thin as possible around
them.

## 9. Analytical Cortex boundary

RFC-009 adds `archie-cortexd` as an unprivileged analytical consumer, not a new kernel
or control-plane authority. Kernel telemetry, PSI/cgroup observations and Sentry records
may become provenance-bearing nodes in a trusted temporal graph, but the Cortex receives
only a policy-filtered immutable snapshot through AF_UNIX IPC. It cannot call cgroup,
eBPF, udev, scheduler, executor or policy interfaces directly.

This keeps the kernel line intact: Linux reports resources and enforces isolation;
`archied`/policy/scheduler decide; Cortex predicts relevance or utility; any resulting
work still follows the ordinary planner -> policy -> scheduler -> executor route. A
neural score is therefore not a resource reservation, lease, capability or command.
