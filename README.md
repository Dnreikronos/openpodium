# OpenPodium

OpenPodium is a free, open-source desktop canvas for running and coordinating coding agents.

The goal is simple: make parallel agent work understandable, recoverable, and safe. Terminals, tasks, notes, Git worktrees, and agent handoffs should live in one local workspace without accounts, telemetry, or a paid feature tier.

> OpenPodium is in its initial development stage. The product contract and architecture are being established before the first usable release.

## Principles

- **Free and open source** — every product feature belongs in the public repository.
- **Local first** — project state remains on the user's machine.
- **Agent agnostic** — Codex, Claude Code, OpenCode, shells, and future CLI tools are adapters rather than hard-coded assumptions.
- **Recoverable** — a crash must not erase the workspace or hide what an agent did.
- **Observable** — tasks, handoffs, failures, retries, and file conflicts are visible.
- **Cross-platform** — macOS, Linux, and Windows are first-class targets.

## Initial scope

The first release will provide:

- An infinite pan-and-zoom canvas.
- Movable and resizable agent terminal nodes.
- Persistent local workspaces.
- Reusable agent roles and command presets.
- Typed agent-to-agent task handoffs.
- Git worktree isolation and collision warnings.
- A durable orchestration timeline with recovery after restart.

See the [product specification](docs/product-spec.md),
[architecture](docs/architecture.md), and
[third-party plugin SDK](docs/plugins.md) for the working contracts. The
[accessibility and localization contract](docs/accessibility.md) documents the
semantic UI model, presentation preferences, international input behavior, and
the remaining native screen-reader limitation.

## Development

OpenPodium is written in Rust. The desktop interface uses Iced and WGPU.

### Requirements

- [Rustup](https://rustup.rs/). The repository pins Rust 1.89.0 and installs the `rustfmt` and `clippy` components automatically.
- **macOS:** Xcode Command Line Tools (`xcode-select --install`).
- **Linux:** a C toolchain, `pkg-config`, a working X11 or Wayland session, and current graphics drivers. Debian and Ubuntu users can start with `build-essential pkg-config`.
- **Windows:** Visual Studio Build Tools with the **Desktop development with C++** workload.

### Run the application

```sh
cargo run
```

### Run the quality gates

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
```

CI runs all three commands on macOS, Linux, and Windows. The current source boundaries are documented in [the architecture](docs/architecture.md#initial-code-organization).

The enforceable trust model, secret-handling rules, and complete outbound
connection inventory are documented in [Security and privacy](docs/security.md).

## License

Licensed under the [Apache License 2.0](LICENSE).
