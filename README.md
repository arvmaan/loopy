# 🛰️ Loopy

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org)
[![Backend: any agent CLI](https://img.shields.io/badge/backend-any%20agent%20CLI-8A2BE2.svg)](#prerequisites)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

**Loopy turns any task into a pipeline of composable blocks, then loops an AI coding agent through them until the work is done.**

> Describe a task → Loopy plans the blocks → you tweak them in the browser → it runs, pausing at review checkpoints.

**[Quick Start](#quick-start)** | **[Prerequisites](#prerequisites)** | **[Blocks](#the-block-library)** | **[Configuration](#configuration)** | **[Architecture](#architecture)**

---

## What is Loopy?

Loopy is a Rust CLI + web app that orchestrates autonomous coding agents. Instead of one giant prompt, every task becomes an ordered **pipeline of blocks** — small, single-purpose steps like *Recon*, *Flight Plan*, *Build*, *Red Team*, *Land*. You describe the task, Loopy proposes a pipeline, you edit it (drag to reorder, add/remove blocks), and it executes — pausing at review checkpoints so you stay in control.

- 🧩 **Composable blocks** — 17 built-in blocks across Understand / Design / Build / Review / Ship. Reorder and mix them per task.
- 🤖 **Agent-agnostic** — works with any agent CLI that runs non-interactively (defaults to `claude`).
- ✋ **Human-in-the-loop** — the pipeline pauses at review gates; approve or send feedback that loops back.
- 🔁 **Resumable** — state is checkpointed to disk; stop the server and pick up where you left off.
- 🌐 **Browser UI** — plan, edit, and watch pipelines live over WebSocket.
- 📂 **Filesystem-native** — Loopy spawns the agent loop; the agent writes JSONL events; Loopy's watcher picks them up. No brittle IPC.

## Prerequisites

| Requirement | Why | Notes |
|---|---|---|
| **[Rust](https://rustup.rs)** (edition 2024) | build the binary | `cargo` |
| **[ralph-cli](https://github.com/mikeyobrien/ralph-orchestrator)** | the loop harness Loopy spawns to run each block | must be on your `PATH` as `ralph` |
| **An agent CLI** | what actually does the work inside the loop | defaults to [`claude`](https://docs.anthropic.com/en/docs/claude-code); set `backend:` in `loopy.yml` for any other |
| **[Node](https://nodejs.org) 18+** | *optional* — only to rebuild the web frontend | the built UI ships embedded in the binary |
| **[`gh`](https://cli.github.com)** | *optional* — only for the pull-request (Land) step | GitHub CLI |

> **ralph-cli is required.** Loopy is an orchestrator — it plans and drives the pipeline, but each block is executed by a [Ralph](https://github.com/mikeyobrien/ralph-orchestrator) loop. Install and authenticate `ralph` (and your agent backend) before running Loopy.

## Installation

```bash
# Clone and build
git clone https://github.com/arvmaan/loopy.git
cd loopy
cargo build --release

# Optional: put `loopy` on your PATH
./scripts/install.sh    # symlinks target/release/loopy into ~/.local/bin
```

## Quick Start

```bash
# 1. Verify your environment (agent backend on PATH, disk space)
loopy doctor

# 2. Bootstrap a project in your repo (.loopy/, loopy.yml, hats)
loopy init

# 3. Start the web UI
loopy start
#    → open http://localhost:3000, describe a task, click "Plan it",
#      tweak the proposed blocks, then "Run pipeline"
```

Prefer to kick off from the command line?

```bash
# Start a pipeline immediately from a task
loopy start "Add rate limiting to the API"

# Custom port, don't auto-open a browser
loopy start --port 3001 --no-open
```

**Resume:** stop the server (Ctrl+C) and restart — Loopy picks up where it left off (state is saved to `.loopy/state.json` every few seconds).

## How it works

Every task becomes a pipeline of **blocks**. A pipeline always starts with a locked **Recon** block and ends with a locked **Land** block; in between, the planner proposes the blocks the task needs and you edit the list before running.

```
Recon → ( Flight Plan · Blueprint · Build · Debrief · Preflight · … ) → Land
          ▲ planned automatically, editable in the UI before you run
```

- **Recon** — the agent researches the task + codebase
- **planned blocks** — the middle of the pipeline, chosen by the planner (or you)
- **review checkpoints** — the pipeline pauses so you can approve or send feedback
- **Land** — produces a summary and (optionally) opens a pull request

## The block library

17 built-in blocks, grouped by phase. Run `loopy blocks` to see them, or `loopy blocks --check` to smoke-test that each generates a valid agent config.

| Phase | Block | What it does |
|---|---|---|
| **Understand** | Recon | Research the codebase and environment; map what exists and what's needed |
| | Intel | Gather external knowledge, prior art, and strategies |
| | Flight Plan | Break the task into a concrete work breakdown / tracks |
| | Black Box | Reproduce the issue and find its root cause before fixing |
| **Design** | Blueprint | Produce a design / spec document before building |
| | Mockups | Produce UI wireframes / mockups for the change |
| **Build** | Build | Do the actual implementation work (may fan out into tracks) |
| | Trim | Refactor for simplicity, then verify behavior is unchanged |
| | Wind Tunnel | Profile and measure performance, optimize, and judge the result |
| **Review & Verify** | Crew Review | Pause for the human team to review and give feedback |
| | Debrief | Standard code review of the produced changes |
| | Red Team | A skeptic agent that tries to refute and break the change |
| | Threat Scan | A security-lens pass (auth, data handling, injection, secrets) |
| | Preflight | Run tests, lint, and build gates; auto-fix mechanical issues |
| **Ship** | Logbook | Update documentation and runbooks |
| | Test Flight | Deploy to a beta/test environment and run E2E checks (never production) |
| | Land | Open the PR and land the work |

## CLI

```
loopy start [IDEA]     Start the web UI (optionally kick off a task)
loopy list             List projects
loopy status           Print pipeline status
loopy init             Bootstrap a project (.loopy/, loopy.yml, hats)
loopy blocks [--check] List pipeline blocks, or smoke-test each block's config
loopy clean            Kill agent loops and remove .loopy/
loopy doctor           Preflight checks (agent backend on PATH, disk space)
```

## Configuration

Loopy reads `loopy.yml` from your project root (`loopy init` creates one):

```yaml
project: MyProject
backend: claude              # any agent CLI on your PATH
max_iterations: 200

# One-shot (non-interactive) invocation used by prompt enrichment + the planner.
# {prompt} is replaced with the instruction. Defaults to `claude -p "<prompt>"`.
agent_oneshot_args: ["-p", "{prompt}"]

# The command agents run to build + test your project, woven into stage prompts
# so the agent verifies its work the way this repo actually builds. When unset,
# prompts fall back to a language-agnostic "detect and run the build/test" hint.
build_command: "cargo test"

# Knowledge base fed to the first (Recon) block
context:
  - type: directory
    path: src/existing-service/
  - type: file
    path: docs/design.md
```

## Architecture

```
loopy start
  └─ axum web server (REST + WebSocket + serves the React UI)
       └─ EngineRunner (background task)
            ├─ Engine        — state machine (phases, checkpoints, feedback loops)
            ├─ Orchestrator  — spawns agent-loop child processes
            ├─ Watcher       — monitors .ralph/events-*.jsonl (notify crate)
            └─ Aggregator    — parses JSONL → pipeline events
```

Key source files:
- `src/pipeline.rs` — blocks, block kinds, the planner, the linear driver
- `src/engine.rs` — the pipeline state machine
- `src/engine_runner.rs` — bridges engine ↔ orchestrator
- `src/orchestrator.rs` — agent process spawning, stage/block dir setup
- `src/web_v2.rs` — REST API + WebSocket
- `web/src/pages/` — the browser UI (React + TypeScript)

## Developing the web frontend

The built frontend is committed to `web/dist/` and embedded into the binary at compile time (`build.rs`). To change the UI:

```bash
cd web
npm install
npm run build                    # rebuilds web/dist
cd .. && cargo build --release   # re-embeds assets
```

## Contributing

Contributions welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Run `cargo test` before opening a PR.

## Acknowledgments

Loopy orchestrates the [Ralph](https://github.com/mikeyobrien/ralph-orchestrator) loop harness — the "keep the agent in a loop until it's done" technique that makes each block work.

## License

[MIT](LICENSE)
