# Versioned plugin SDK and third-party adapters

Status: Issue #23 implementation contract  
Updated: 2026-09-21

## Boundary

Plugins are external executables discovered from user-configured directories.
OpenPodium never loads third-party native code into its process. Each enabled
plugin runs as a child process with piped standard input and output, so a panic,
abort, malformed response, or incompatible SDK cannot unwind through the host
or mutate the in-memory workspace.

The plugin layer owns manifests, SDK negotiation, contribution discovery,
permission decisions, process supervision, and the wire protocol. It returns
validated data to the application layer; it does not apply domain commands,
write workspace snapshots, or launch an adapter-produced command by itself.

This boundary isolates failures, not arbitrary operating-system access. Native
plugins run with the account's ambient filesystem and network permissions.
Capabilities control which OpenPodium APIs are exposed. A future WebAssembly
runner can provide a stronger operating-system sandbox while retaining the same
manifest and protocol.

The child environment is cleared before launch. OpenPodium forwards only basic
platform variables needed to execute programs (`PATH`, temporary-directory,
Windows system-path, and locale variables) plus the plugin ID and negotiated
SDK version. Host credentials and application-specific environment variables
are not inherited implicitly.

## Manifest and compatibility

Every plugin directory contains `plugin.json`. Identifiers use reverse-DNS
notation and are stable across updates. The manifest declares its plugin
version, the inclusive SDK range it supports, requested capabilities, an
executable path relative to the manifest, and its contributions.

OpenPodium SDK versions use `major.minor`. A plugin activates only when the
host version falls inside its inclusive range. Major versions define wire and
semantic compatibility; minor versions add optional fields or operations. The
negotiated version is the host version. Unknown manifest fields are ignored so
older hosts can inspect newer manifests, but unknown capabilities and duplicate
contribution identifiers are rejected.

## Capabilities and permissions

The stable capabilities are:

- `adapters`: declare providers and answer adapter preparation requests.
- `commands`: declare commands and receive command invocations.
- `events`: receive explicitly forwarded host events.
- `settings.read`: receive namespaced plugin settings.
- `settings.write`: propose namespaced plugin setting changes.
- `ui`: declare host-rendered panels and settings fields.

A contribution requires its matching declared capability. At activation, a
permission policy intersects declared capabilities with the user's grants. The
host sends only that intersection in the handshake and rejects protocol
operations outside it. Settings are always namespaced by plugin identifier.
Plugins never receive direct references to workspace state or persistence.

## Contributions

Providers and commands are declarative manifest entries with plugin-local
identifiers and user-facing labels. Providers describe the models they expose;
the runtime request asks the plugin to return an argument-vector launch plan.
Commands receive bounded JSON context and return a typed result for the
application to interpret. Event subscriptions list stable event names.
Settings fields and UI panels are schemas rendered by OpenPodium; plugins do
not inject native widgets or execute code on the UI thread.

Global contribution identifiers are qualified as `<plugin-id>/<local-id>`.
The catalog rejects duplicates within one manifest and keeps contributions from
disabled or incompatible plugins out of the active registry.

## Discovery, lifecycle, updates, and diagnostics

Discovery scans direct child directories for `plugin.json`, validates each
manifest, resolves its executable without searching `PATH`, and records one
diagnostic per rejected candidate. Duplicate plugin identifiers are rejected
deterministically so directory order cannot silently replace code.

Plugins start disabled until explicitly enabled. Enabling revalidates SDK
compatibility and permissions before the process is spawned. Disabling stops
the child and removes its active contributions. Refreshing discovery is the
update flow: a changed manifest replaces the previous version only after full
validation, while the prior valid record remains available if the candidate is
invalid. Permission grants are reevaluated on every activation.

Diagnostics expose discovery, compatibility, permission, startup, protocol,
and exit failures without including standard input, settings values, or other
workspace content. A failed session is quarantined; callers receive a typed
error and may explicitly restart it.

## Wire protocol

Standard input and output carry one UTF-8 JSON object per line. Standard error
is inherited for developer diagnostics and is never parsed. The host begins
with `host_hello`, containing the negotiated SDK and granted capabilities. The
plugin must answer with a matching `plugin_ready` before requests are sent.

Every later request has a monotonically increasing numeric ID. Responses must
echo it. Unknown message types, oversized lines, invalid JSON, mismatched IDs,
EOF, and response timeouts are failures that terminate and quarantine only that
session. Messages are limited to 1 MiB and the default response timeout is 30
seconds; hosts can choose a shorter timeout for latency-sensitive calls. The
initial implementation supports command invocation, adapter preparation, event
delivery, settings reads, and setting-change proposals.

## Sample plugin

`src/bin/openpodium-sample-plugin.rs` is an independent executable that depends
only on the public `openpodium::plugins` wire types. Its manifest lives at
`examples/sample-plugin/plugin.json` and contributes the `echo` provider plus
the `say-hello` command. Build it with:

```sh
cargo build --bin openpodium-sample-plugin
```

Copy the resulting binary beside `plugin.json` (add `.exe` to both the binary
and the manifest path on Windows), then pass the containing directory to
`PluginCatalog::discover`. Newly discovered plugins are disabled. Grant only
the capabilities the user approved, enable the catalog record, and start its
`PluginSession`; the catalog exposes only contributions backed by granted
capabilities.

## Verification

Targeted tests cover manifest validation, SDK range negotiation, contribution
capability requirements, permission intersection, deterministic discovery,
enable/disable and refresh behavior, protocol authorization, successful sample
provider and command calls, incompatible SDK rejection, and recovery from a
plugin that exits or emits malformed data. The checked-in sample plugin is an
independent executable using only the public wire types.
