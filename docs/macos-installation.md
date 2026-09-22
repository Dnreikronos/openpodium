# macOS application bundle

Package OpenPodium as a native `.app` with a stable bundle identifier and a blue
rounded-square O icon. Installing the bundle in `/Applications` makes it available
to Finder, the Dock, and Spotlight. A launcher supplies a login-shell environment
so installed agent commands remain discoverable when launched without a terminal.

Run `bash scripts/package-macos.sh` on macOS to build the optimized application.
Use `bash scripts/package-macos.sh --debug` for a local development build. The
script prints the bundle location in a fresh directory under `target/macos` and
never overwrites an installed app. It generates all icon resolutions from the
vector drawing in `packaging/macos/generate-icon.swift` and signs the local bundle
ad hoc. Public distribution will require Developer ID signing and notarization.

Quit the previous instance before copying `OpenPodium.app` into `/Applications`.
Default data lives in `~/Library/Application Support/OpenPodium`; the app bundle
contains no workspaces. Back up existing data before migrating another profile,
including SQLite WAL files, attachments, and project folders. Update moved
workspace and floor paths through domain commands rather than rewriting journal
history. Keep the source profile until the installed app is verified.

Verify the bundle metadata, icon representations, signature, launch environment,
saved workspace recovery, and a single running instance. Search for OpenPodium
with Command-Space and open that result to verify Spotlight registration.
