# Security, privacy, and local trust boundaries

Status: Issue #24 security contract
Updated: 2026-09-21

## Trust model

OpenPodium is local-first, but not every local input is trusted. Repository
contents, Git output, terminal byte streams, IPC frames, portable documents,
portal pages and device trees, plugin manifests, and plugin protocol messages
are attacker-controlled inputs. Workspace state, IPC credentials, project
files outside an explicitly selected checkout, and user approval decisions are
protected assets.

The operating-system account is the outer security boundary. Local agent CLIs
and native plugins execute as that account and are not sandboxes: they can use
the account's ambient filesystem and network access independently of
OpenPodium. Capability grants restrict OpenPodium APIs, not operating-system
access. Run untrusted native code in an OS sandbox, container, VM, or separate
account when that distinction matters.

Other processes running as the same user may inspect process environments on
some platforms and can probe loopback ports. OpenPodium therefore keeps secrets
out of argument vectors where possible, uses short-lived session-bound IPC
tokens, binds services to loopback, and treats the Chromium CDP and Appium
connections as local privileged surfaces. These mitigations do not create a
same-user process sandbox.

## Boundary controls and verification

| Boundary | Enforced policy | Code | Targeted evidence |
|---|---|---|---|
| Process launch | Programs and arguments remain separate; no general-purpose shell interpolation. Container environment values are inherited by name instead of appearing in the engine command line. SSH is the one string-protocol exception and quotes every token. | `src/runtime.rs`, `src/runtime/adapters.rs`, `src/runtime/environments.rs`, `src/runtime/local.rs` | `runtime::tests`, `runtime::environments::tests` |
| Terminal output | ANSI bytes are display input, never task state or commands. Scrollback is bounded to 10,000 lines. | `src/terminal.rs`, `src/orchestration` | `terminal::tests::arbitrary_terminal_output_preserves_render_invariants`, terminal rendering tests |
| Local IPC | IPv4 loopback only; per-install 256-bit secret; per-process session binding; workspace and agent scoped tokens; constant-time comparison; generic authorization failure; bounded newline frames and I/O timeouts. The secret must be a regular 0600 file on Unix and cannot be a symlink. | `src/ipc/auth.rs`, `src/ipc/client.rs`, `src/ipc/protocol.rs`, `src/ipc/server.rs` | `ipc::auth::tests`, `ipc::server::tests`, `ipc::protocol::tests::arbitrary_protocol_input_never_bypasses_typed_validation`, `tests/ipc_cli.rs` |
| Portable imports and exports | Versioned allowlists, unknown-field rejection, document/item bounds, symbolic IDs, symlink-aware project paths, atomic import, and export blocking for likely secrets. Machine paths, environments, terminal state, ownership tokens, and runtime handles are omitted. | `src/persistence/portable.rs`, `src/workspaces.rs` | `persistence::portable::tests::arbitrary_portable_documents_are_rejected_or_fully_validated`, secret and path import/export tests |
| Git and repository paths | Commands use argument arrays, `--` path separators, canonical checkout identity, and explicit worktree ownership. External stderr is redacted before display. | `src/git.rs`, `src/git/changes.rs`, `src/git/context.rs`, `src/git/integration.rs` | `git::tests`, `git::context::tests`, `git::integration::tests` |
| Browser portals | Only HTTP(S) navigation; isolated temporary Chromium profile; Chromium chooses an ephemeral loopback CDP port; observation-revision element tokens; explicit policy approval and expiring grants for consequential actions. | `src/portal/browser.rs`, `src/portal/policy.rs`, `src/ipc/portal.rs` | browser, portal policy, and IPC portal tests |
| Device portals | Appium endpoints must be loopback, responses are capped at 8 MiB, XML identities are observation-scoped, and consequential actions pass through the same approval policy. | `src/portal/appium.rs`, `src/portal/device.rs`, `src/portal/policy.rs` | `portal::appium::tests::arbitrary_appium_xml_and_http_responses_fail_safely`, Appium identity and policy tests |
| Native plugins | Disabled by default; manifest paths cannot traverse or escape through symlinks; child environment is cleared; protocol frames are capped at 1 MiB; timeouts, ID matching, and quarantine contain protocol failures. Grants are intersected with declarations. | `src/plugins/catalog.rs`, `src/plugins/manifest.rs`, `src/plugins/protocol.rs` | `plugins::tests::arbitrary_manifest_input_is_rejected_or_fully_validated`, `tests/plugins.rs` |
| Remote devices | Explicit single-use local pairing; X25519 identity agreement; XChaCha20-Poly1305 payload encryption; revocable workspace and operation grants; bounded packets; ordered acknowledgements and idempotent command IDs. Relays receive only authenticated ciphertext and routing metadata. | `src/remote` | `remote::tests` |
| Diagnostics | Common assignments, authorization headers, credential prefixes, URL passwords, and private keys are redacted. Secret-bearing process and adapter plans have redacted `Debug` output. OpenPodium has no crash uploader or remote logging sink. | `src/security.rs`, process/plugin/Git/portal error boundaries | `security::tests`, `runtime::tests::process_debug_output_redacts_secret_arguments_and_environment`, plugin redaction tests |

Terminal content and project files are intentionally user-visible and may
contain secrets. The guarantee above concerns workspace exports, structured
diagnostics, debug formatting of secret-bearing launch plans, and data sent
across OpenPodium-owned protocols. It is not a promise to hide a secret that a
child process deliberately prints in its terminal.

## Plugin permission review

`PluginRecord::permission_review` and `PluginCatalog::permission_review`
provide a host-renderable review surface before activation. It lists every
requested capability, whether it is granted, an observe/change/execute impact,
and a mandatory notice that native plugins retain the user's ambient
filesystem and network access. Enabling still rejects grants the manifest did
not request; disabling clears every grant.

## Network and telemetry inventory

OpenPodium contains no telemetry client, analytics SDK, update checker, crash
uploader, account service, or remote logging transport. It makes no network
connection merely by opening a workspace. Every network-capable component is
listed below.

| Component | Destination | Trigger and data | Policy |
|---|---|---|---|
| IPC service/client | Ephemeral `127.0.0.1` port | App start and commands from launched local agents; typed workspace coordination plus scoped credential | Loopback-only and authenticated |
| Chromium portal | User-entered HTTP(S) origin and its page subresources | Explicit portal connection/navigation; normal browser traffic and page input | URL scheme restriction plus portal approvals; isolated temporary profile |
| Chromium CDP client | Ephemeral loopback port selected by Chromium | While a browser portal is connected; screenshots, accessibility trees, and approved input | Loopback-only; no remote CDP endpoint accepted |
| Appium portal/probe | `127.0.0.1:4723` or another loopback test endpoint | Explicit device connection or discovery; WebDriver commands, screenshots, and page source | Non-loopback addresses rejected; bounded responses; portal approvals |
| SSH environment | User-configured SSH host and port | Explicit health check or agent launch; quoted remote command and normal SSH authentication handled by the system client | No password/key persistence; batch-mode health checks; IPC token is not forwarded |
| Container environment | User-configured Docker/Podman engine | Explicit health check or agent launch; inspect/exec requests. The engine may itself use a configured local or remote daemon. | Argument arrays; environment values kept out of the engine command line |
| Device discovery | Local `adb`, `xcrun`, and Appium tooling | Explicit discovery; tool-defined device/daemon traffic | Output is parsed as untrusted input; diagnostic stderr is redacted |
| Agent and custom runtime processes | Process-defined destinations | Explicit agent launch | Outside OpenPodium's network mediation; isolate with OS facilities when required |
| Native plugin processes | Process-defined destinations | Explicit enable/start after permission review | Outside OpenPodium's network mediation; child environment is cleared but OS network access remains |
| Remote relay or direct peer | User-configured HTTPS/WebSocket relay or paired peer | Explicit remote-access enablement; encrypted remote packets plus routing metadata | Project content is encrypted end to end; device grants are checked locally; relay transport is not yet selected |

Git operations in this repository are local status, diff, worktree, merge,
rebase, and cherry-pick operations; OpenPodium does not invoke fetch, pull,
push, or clone. SQLite persistence, file indexing, image decoding, desktop
notifications, and orchestration do not open network connections. The
`tungstenite` dependency is used only for the loopback Chromium CDP connection.

## Dependency audit

`cargo audit` against the 2026-09-21 RustSec database reports no known
vulnerabilities in `Cargo.lock`. It reports four unmaintained transitive crates
(`bincode`, `paste`, `ttf-parser`, and `yaml-rust`) and the informational
unsoundness advisory RUSTSEC-2026-0253 for `lru 0.16.4`, pulled in by Iced's
`cryoglyph` renderer. The advisory requires a cache key with a panicking
`Drop` implementation plus caught unwinding; `cryoglyph` uses
`cosmic_text::CacheKey` in an internal glyph cache, so that trigger is not
present in this use. Upgrade through Iced when its renderer accepts the patched
`lru >=0.18.2`; do not force a semver-incompatible transitive override.

## Adding a new boundary

A change that parses external data, launches a process, persists or exports
content, or opens a socket must update this document, define size/time/path and
authorization policies, keep secrets out of arguments and diagnostics, and add
a targeted adversarial or property test. A new outbound destination must be
added to the network inventory before release.
