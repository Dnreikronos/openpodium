# Runtime environment contract

Status: Issue #9 implementation contract  
Updated: 2026-09-19

## Boundary

Runtime environments describe where a terminal command runs. A workspace owns
durable environment profiles, and an agent may select one profile. Agents that
do not select a profile use the workspace-local environment so snapshots from
older versions retain their current behavior.

Profiles contain launch metadata only. Live processes, PTYs, terminal emulator
state, health probes, and reconnect attempts remain ephemeral runtime state.
Environment launchers translate a profile and an argument-vector command into
the existing `ProcessSpec`; every resulting process therefore uses the same
input, output, resize, cancellation, and exactly-once termination contract.

## Profile kinds

- **Local** runs the command directly in the workspace directory. It is the
  implicit default and is not stored as a user-created profile.
- **SSH** stores host, optional user and port, and a remote working directory.
  It launches the installed OpenSSH client with an argument vector. OpenSSH
  configuration, host keys, `ssh-agent`, and operating-system credential
  integration remain authoritative for authentication.
- **Container** stores a Docker-compatible CLI executable, an existing
  container name or ID, and a container working directory. It uses `exec`; it
  never creates, starts, stops, or removes a container.
- **Custom** stores a local executable, prefix arguments, and a local working
  directory. The terminal command is appended as distinct arguments. Shell
  command strings and interpolation are not supported.

The first SSH implementation targets POSIX remote hosts because OpenSSH sends a
remote command through the account's login shell. OpenPodium shell-quotes each
remote argument and never embeds authentication material in that command.

## Validation and health

Profile construction validates names, hosts, ports, container identifiers,
working directories, executables, and argument counts before persistence.
Local and custom directories are also checked through the filesystem before a
launch. Remote checks run with a short timeout and without interactive
authentication:

- SSH checks the connection and remote directory with `BatchMode=yes`.
- Container checks that the engine can inspect the container and that the
  configured directory exists inside it.
- Custom checks executable resolution and local directory access without
  executing the configured command.

Health is an ephemeral observation: `unknown`, `checking`, `healthy`,
`missing executable`, `authentication failed`, `unreachable`, or `invalid
working directory`. A failed health probe never mutates durable configuration.

## Session identity and reconnect

A live terminal is identified by `(workspace ID, node ID)`. Switching the
visible workspace detaches its view but does not stop its process. Returning to
the workspace reuses the existing session and scrollback. A start or reconnect
request for an identity whose process is already starting or running focuses
that session and does not spawn another process.

If the transport exits, the terminal retains its final output and reports the
environment health or process failure. Reconnect is an explicit new launch only
after the previous process has terminated. This issue does not install or
manage a remote supervisor such as `tmux`, so transport loss cannot reattach to
a remote process that outlived its SSH connection.

Removing a node, explicitly stopping a terminal, or closing OpenPodium cancels
the associated process. Removing a profile that is still referenced by an
agent is rejected.

## Security and persistence

Workspace snapshots and events persist profile metadata and agent references.
They never persist passwords, private keys, passphrases, access tokens, Docker
credentials, or live handles. SSH and container authentication is delegated to
the installed clients and their operating-system-backed credential helpers.
Custom profile fields are ordinary persisted configuration and therefore must
not contain secrets.

Authentication failures are mapped to a stable redacted error category. UI
errors may name the profile and failed operation, but do not echo process
arguments, environment variables, credential-helper output, or raw
authentication prompts.

Container launches pass `--env NAME` and place the corresponding value only in
the engine process environment. This keeps values out of the Docker or Podman
argument vector and therefore out of ordinary process listings. Remote SSH
commands cannot inherit a local environment directly, so only the explicitly
prepared non-secret agent environment is shell-quoted into that protocol.

## Verification

Targeted domain and persistence tests cover profile validation, references,
event replay, legacy snapshot defaults, and close/reopen round trips. Runtime
tests use fake SSH and container-compatible executables to verify generated
argument arrays, health classification, redaction, and working-directory
checks without requiring network or daemon access. Application tests verify
that workspace switching preserves sessions and that repeated start or
reconnect requests do not duplicate a live process.
