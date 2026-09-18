# OpenPodium product specification

Status: Draft 0.1  
Updated: 2026-09-18

## Product statement

OpenPodium is a free, local-first desktop workspace where developers can arrange coding agents on a spatial canvas, give them isolated tasks, observe their work, and coordinate handoffs without losing project context.

OpenPodium provides the orchestration environment. It does not provide or resell AI models; users install and authenticate their preferred coding-agent CLIs separately.

## Problem

Running several coding agents in ordinary terminal tabs creates three recurring failures:

1. The developer becomes the message bus, manually moving context and results between agents.
2. Parallel agents edit overlapping files without knowing they are colliding.
3. Terminal state explains activity poorly and is difficult to reconstruct after interruption or restart.

A visual canvas helps with spatial awareness, but the product must also make orchestration durable and deterministic. OpenPodium treats tasks, agent identities, handoffs, and worktree ownership as structured state instead of inferring all meaning from terminal output.

## Goals

- Show the complete state of multi-agent work at a glance.
- Run existing agent CLIs without locking users to a model provider.
- Preserve workspace and task state across crashes and restarts.
- Isolate parallel changes with Git worktrees.
- Warn before agents with overlapping write sets are merged.
- Make every delegation and response inspectable.
- Remain useful offline, apart from network access required by an agent CLI.
- Keep all OpenPodium features free and in the public repository.

## Non-goals for the first release

- Shipping an OpenPodium-hosted model or proxying model billing.
- Replacing a full code editor.
- Remote team collaboration or cloud synchronization.
- Mobile clients and device/browser automation portals.
- Reproducing every feature or visual detail of another product.
- Autonomous merging without user review.

## Primary user

A developer who works on a Git repository with two or more coding agents and needs to divide implementation, review, testing, or research work without managing a pile of terminal windows.

## Core concepts

- **Workspace** — a project directory and its saved canvas, agents, tasks, and settings.
- **Canvas** — the spatial surface containing nodes and connections.
- **Agent** — a named CLI process with a role, command preset, working directory, and lifecycle state.
- **Role** — reusable instructions such as Lead, Builder, Reviewer, or Tester.
- **Task** — a durable unit of work with an owner, status, prompt, responses, and optional worktree.
- **Handoff** — a structured task or question sent from one agent to another.
- **Worktree** — an isolated Git checkout assigned to a task or agent.
- **Timeline event** — an immutable record of a meaningful state change.

## First-release capabilities

### Workspace and canvas

- Create a workspace from a local directory.
- Pan and zoom without a fixed canvas boundary.
- Create, move, resize, connect, and remove nodes.
- Restore the exact canvas layout after restarting.
- Navigate entirely with mouse/trackpad or keyboard.

### Terminals and agents

- Run an interactive local shell inside a node.
- Launch Codex, Claude Code, OpenCode, or a custom command.
- Assign a name, color, icon, and reusable role.
- Display starting, running, waiting, completed, failed, and stopped states.
- Preserve scrollback and enough launch metadata to recover a session safely.

### Orchestration

- Send a typed task or question to another connected agent.
- Track queued, delivered, running, blocked, completed, failed, and cancelled states.
- Display the originating agent, receiving agent, timestamps, and response.
- Permit retries without overwriting the failed attempt.
- Never require a terminal to remain unfocused for a handoff to complete.

### Git safety

- Create an optional worktree and branch for an assigned task.
- Show changed files for every active worktree.
- Warn when active agents modify overlapping paths.
- Require the user to choose how changes are integrated.

### Recovery and privacy

- Persist state continuously rather than only on clean shutdown.
- Restore a workspace after forced termination.
- Store data locally using documented formats and SQLite.
- Send no telemetry and require no OpenPodium account.

## Acceptance criteria for 0.1

The release is usable when a developer can open a Git project, place two agent terminals on the canvas, assign isolated worktrees, delegate a task from one agent to the other, observe the task lifecycle, restart OpenPodium, and recover the layout and orchestration history without losing completed work.

Targeted automated tests must cover domain transitions, persistence recovery, PTY lifecycle behavior, and Git collision detection. CI must build and test macOS, Linux, and Windows before 0.1 is tagged.

## Later opportunities

- Notes, diagrams, file trees, and browser/device portals.
- Saved workspace templates.
- Remote monitoring and collaboration.
- Plugin SDK and third-party agent adapters.
- Sandboxed execution policies.
- Optional local summarization models.

