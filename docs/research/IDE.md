# Which IDE, and how Archie works inside one

Surveyed 2026-09-13 and 2026-09-14. Package versions and licences were read from the
archlinux.org package pages on those dates. Protocol support was read from each editor's
own documentation and the protocols' published specifications.

Two questions, with different answers:

1. **What should ArchieOS ship as its editor?** Zed, with Code - OSS offered alongside.
2. **How does Archie collaborate with a developer's IDE, including Claude Code running in
   VS Code?** Through the Model Context Protocol: `archie-mcp-server` is a Rust binary any
   MCP-capable IDE starts over stdio. No IDE extension is needed, so nothing about it is
   TypeScript or Python.

## The editor on the ISO: Zed

| | Zed | Code - OSS (`code`) | Visual Studio Code | VSCodium |
|---|---|---|---|---|
| Arch package | `zed`, extra, 1.19.2 | `code`, extra, 1.137.0 | `visual-studio-code-bin`, AUR | `vscodium-bin`, AUR |
| Licence | GPL-3.0-or-later; AGPL-3.0-or-later in Arch's listing | MIT | Microsoft product licence | MIT |
| Written in | Rust, GPU-rendered | TypeScript on Electron | TypeScript on Electron | TypeScript on Electron |
| MCP servers | yes (`context_servers`) | through whichever agent extension is installed | yes: Copilot agent mode, GA since 1.102 (July 2025) | through extensions |
| Agent Client Protocol | native (Zed created it) | third-party extensions only | third-party extensions only | third-party extensions only |
| Marketplace | Zed extensions | Open VSX | Microsoft Marketplace | Open VSX |

Zed wins for the ISO on three counts:

- **It fits the machine.** A Rust editor rendering on the GPU starts fast and idles cheaply
  on a node that is also running models. An Electron editor is a browser.
- **It speaks both agent protocols natively.** MCP servers for tools, and the Agent Client
  Protocol for agents that live inside the editor. So Archie can be both the editor's tool
  provider and, later, an agent in its agent panel.
- **It ships from Arch's official repositories** under a free licence, so it can be
  preloaded without an AUR build in the image pipeline.

Offer Code - OSS next to it for people whose extensions only exist in the VS Code world.
The Microsoft build is not redistributable in an image. Its Marketplace terms also limit
extensions to Microsoft's own products, so Code - OSS uses Open VSX.

Zed's collaboration features, shared projects and calls, need a zed.dev sign-in. The
editor itself does not.

## Archie as a collaborator: MCP, not an extension

The request was for Archie to join a developer's VS Code "as a collaborator". Three ways
were considered.

**Live Share: not viable.** Microsoft's Live Share runs only in Microsoft's builds of VS
Code, needs a Microsoft or GitHub account, and has no programmatic way for a process to
join a session. An agent would have to drive a signed-in editor's UI to take part.

**A VS Code extension: not needed, and against the project rule.** Extensions are
JavaScript or TypeScript. An extension would also only reach VS Code, while MCP reaches
every IDE agent at once.

**MCP: the chosen path.** VS Code's agent mode, Claude Code (in a terminal, or in its VS
Code and JetBrains extensions), Zed, Cursor and others all start MCP servers and call
their tools. So `archie-mcp-server` gives each of them the same tools, from one Rust binary.

### What the server offers

| Tool | Grade | Effect | What it returns |
|---|---|---|---|
| `archie_ping` | public | read-only | whether `archied` answers, and its version |
| `archie_status` | internal | read-only | health, version, OS, CPU, memory, accelerators, storage, capabilities, counts. **Not** the hostname, network interfaces or location |
| `archie_modules` | internal | read-only | installed modules, their state and why, dependencies, load order |
| `archie_providers` | internal | read-only | data providers, whether each runs and why, known blind spots |
| `archie_models` | internal | read-only | registered models, locality, availability, privacy clearance |
| `stage_surfaces` | internal | read-only | applications on the Archie Stage, their geometry, who holds the stage |
| `stage_act` | internal | **acts** | click, drag, scroll, type or press keys in a stage application |

**The server is a disclosure path.** Whatever a tool returns goes into the IDE agent's
context, and from there to that agent's model, which Archie cannot see or verify. So each
tool declares the most sensitive grade its result can carry. The server runs with a
clearance, and tools above it are neither listed nor callable. The default clearance is
`public`, which exposes only `archie_ping`. `--clearance internal` is for an operator who
has checked the IDE model's retention, and it is the ceiling: there is no flag for
sensitive data. `archie_status` builds its result from an allowlist of fields, so a field
added to the status later does not leave the node until someone decides it should.

**Actions are opt-in and go through the gate.** `stage_act` exists only when the server is
started with `--allow-stage-actions` and the stage key. Each call goes through the same
`archie_stage::Operator` as the stage CLI. That means the stage policy and a one-operation
lease for the IDE agent, which is its own principal and is never taken for a person at the
machine, then the gate, sealing, and the compositor's own checks. Anything the policy
holds for approval is refused, since the server has no way to ask a person. A person
touching the mouse or keyboard takes the stage back. The agent sees that as an error
result, and no tool can clear it.

### Configuration

The server runs where `archied` runs. For an IDE on another machine, run it over SSH:
MCP over stdio works through `ssh node archie-mcp-server …` without anything else.

**Claude Code** (terminal, or its VS Code extension, which reads the same configuration):

```bash
claude mcp add archie -- archie-mcp-server --clearance internal
```

or, shared with a repository, `.mcp.json` at its root:

```json
{
  "mcpServers": {
    "archie": {
      "command": "archie-mcp-server",
      "args": ["--clearance", "internal"]
    }
  }
}
```

**VS Code** (agent mode), `.vscode/mcp.json`:

```json
{
  "servers": {
    "archie": {
      "type": "stdio",
      "command": "archie-mcp-server",
      "args": ["--clearance", "internal"]
    }
  }
}
```

**Zed**, `settings.json`:

```json
{
  "context_servers": {
    "archie": {
      "source": "custom",
      "command": "archie-mcp-server",
      "args": ["--clearance", "internal"],
      "env": {}
    }
  }
}
```

Letting an IDE agent operate stage applications adds, on the node:

```bash
archie-mcp-server --clearance internal --allow-stage-actions --stage-key /etc/archie/stage/key --stage-policy /etc/archie/stage/apps.toml
```

### Protocol notes

- It speaks MCP revisions 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05, and answers
  an unknown revision with the newest of these. Only `tools/list` and `tools/call` with
  text content are used, and they mean the same in all four.
- One JSON-RPC object per line. Batches are refused, since 2025-06-18 removed them.
  Messages over 1 MiB are refused and skipped to their end, so the next message still
  frames. Tool results over 512 KiB are errors, not truncations.
- A hidden tool and a missing tool get the same error. Method names are not echoed back.
- Standard output carries protocol messages only; the log goes to standard error.

## Later: Archie as an agent inside the editor

The **Agent Client Protocol** (ACP, published by Zed in August 2025; not to be confused
with Iris's Archie Context Protocol) runs the other way round. The editor is the client,
and an agent process runs in its agent panel with the editor's files, diffs and terminals.
Zed implements it natively, JetBrains IDEs have adopted it, Neovim and Emacs have plugins,
and VS Code has it only through third-party extensions. An official Rust SDK exists
(`agent-client-protocol` on crates.io).

An `archie-acp-agent` would put Cerebra behind the editor's agent panel. It is not built
here. Before it is:

- Listing in the public ACP registry requires the agent to support authentication. Archie's
  answer to that is the vault's credential broker, and it needs designing, not a stub.
- The agent would read and propose edits to a workspace, so it belongs behind the
  executor's workspace capability, not beside it.
- Zed can run it without the registry, as a custom agent server:

```json
{
  "agent_servers": {
    "archie": {
      "type": "custom",
      "command": "archie-acp-agent",
      "args": [],
      "env": {}
    }
  }
}
```

## Not verified

- No real IDE has been pointed at `archie-mcp-server` yet, and no node with `archied`
  running has served it. What ran was the protocol tests, then a simulated IDE sending
  JSON-RPC over the server's stdin in WSLg:
  - At `public` it saw only `archie_ping`, which reported that `archied` was not running.
  - At `internal` with stage actions it read `stage_surfaces`, then clicked, typed and saved
    the demo form through `stage_act`. The form logged "Hola desde el IDE, año 2026".
  - An application with no surface was refused, and so was `super+l`, as a system chord.
- Zed's `context_servers` and `agent_servers` keys are as Zed documented them on the
  survey date. Zed changes settings formats between releases, so check its documentation
  if a key is rejected.
