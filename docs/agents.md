# Agent adapters, command presets, and reusable roles

Status: Issue #10 implementation contract
Updated: 2026-09-19

## Boundary

Agent launch configuration is durable workspace state. A workspace owns custom
command presets and roles; an agent references one built-in adapter or custom
preset plus an optional role. Import and export use a versioned portable JSON
document so a role can be copied between workspaces without introducing global
configuration storage.

Adapters translate durable launch configuration into the runtime's existing
argument-vector `ProcessSpec`. They do not spawn processes, mutate workspace
state, write instruction files into repositories, or concatenate a shell
command. Environment profiles continue to decide where the resulting process
runs.

Changes to an agent's role or launch selection apply the next time its terminal
starts or reconnects. OpenPodium never attempts to replace instructions inside
an already-running CLI session.

## Launchers and instruction injection

The built-in launchers are Codex (`codex`), Claude Code (`claude`), OpenCode
(`opencode`), and the platform login shell. A custom preset stores a name,
executable, and ordered argument array. Preset values are validated and passed
directly to the process API; they are never parsed as a shell command.

Every launch includes `OPENPODIUM_ROLE_INSTRUCTIONS` when a role is assigned.
Adapters additionally use the CLI's supported instruction mechanism:

- Codex passes a `developer_instructions` configuration override.
- Claude Code passes `--append-system-prompt`.
- OpenCode defines and selects an ephemeral primary agent through
  `OPENCODE_CONFIG_CONTENT`.
- Shell and custom presets use only the generic environment variable because
  they have no portable system-prompt interface.

The generated executable, argument array, and environment overrides are exposed
as a preview before launch. User-controlled values remain individual arguments
or environment values throughout local, container, and custom environments.
SSH remains the one transport boundary that serializes arguments for the remote
login shell, using the quoting contract defined by issue #9.

## Roles

A role contains an ID, name, `#RRGGBB` display color, non-empty icon label, and
instructions. Roles can be created and edited, assigned to an agent, replaced
with another role, or cleared. Removing a role that is still assigned is
rejected so agent references cannot become dangling.

Portable role JSON has this shape:

```json
{
  "format": "openpodium-role",
  "version": 1,
  "role": {
    "name": "Reviewer",
    "color": "#8B5CF6",
    "icon": "review",
    "instructions": "Review changes for correctness and regressions."
  }
}
```

IDs are deliberately omitted. Import validates every field and assigns a fresh
ID in the destination workspace, so importing never overwrites an existing
role implicitly.

## Capabilities and setup guidance

Capability detection resolves the selected executable without invoking it.
Built-ins report whether their expected binary is available; custom presets do
the same for their configured executable. A missing executable prevents launch
and returns adapter-specific installation guidance instead of allowing a PTY
spawn failure to surface as an unexplained crash.

Capability checks are observations, not durable state. Authentication and
provider availability remain owned by each CLI.

## Persistence and compatibility

Command presets, role appearance, agent launch selection, and agent role
changes are journaled and included in snapshots. Older snapshots and events
retain their existing defaults: shell remains the default launcher, existing
roles receive the standard color and icon, and absent preset collections decode
as empty.

## Verification

Targeted tests cover domain validation and reference safety, role assignment
changes, versioned role import/export, event and snapshot round trips, legacy
defaults, exact adapter argument/environment previews, hostile values remaining
single arguments, executable detection, and user-facing missing-tool guidance.
