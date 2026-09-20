# Portal implementation

Issue #20 is split into phases so the live automation path does not become part
of durable canvas state.

## Phase 1: browser interaction kernel

The first slice is a dependency-free contract in `src/portal.rs`. It defines
the durable target and presentation intent, capability reporting, observation
revisions, semantic element references, session lifecycle, and the geometry
needed to map canvas pointer coordinates into a portal viewport. The geometry
tests cover zoom-independent pointer mapping, aspect-ratio letterboxing, and
stale observation references.

This phase is fixture-backed because no Chromium executable is available on the
development machine. It proves the state and coordinate rules before adding a
CDP client or an iced image surface.

## Phase 2: durable canvas content and rendering

Add `CanvasNodeContent::Portal(PortalConfig)` with backward-compatible journal,
canvas-fragment, template, and workspace-archive decoding. A portal copy or
import starts disconnected. Render the most recent bounded frame inside the
existing canvas scene. The phase 1 geometry handles aspect-ratio-correct frame
placement and provides the coordinate mapping needed by the remaining pointer
and scroll input work.

## Phase 3: browser adapter and policy

Add an isolated Chromium/CDP adapter and a dedicated portal dispatcher. Every
agent request resolves the authenticated workspace connection and current node
configuration, checks policy, then dispatches against the current observation
revision. Input, navigation, coordinate fallback, downloads/uploads,
clipboard, and sensitive browser permissions remain separate policy decisions.

Journal intent before dispatch and return an unknown outcome after a crash that
may have occurred after dispatch. Screenshots are bounded authenticated
transfers and are not journal payloads.

## Phase 4: devices and lifecycle

Add Appium adapters behind the same contract, then cover discovery failures,
device removal, reconnect, close during connect/action, process ownership, and
application shutdown. Forwarding for remote/container agents remains separate
because those agents currently receive no IPC endpoint.

The product specification still lists portals outside the first release; this
work therefore remains behind explicit portal configuration and does not change
the release claim until the full acceptance matrix passes.

## Current desktop flow

The app can create a browser portal node from a URL, explicitly connect an
isolated Chromium process, and capture bounded frames off the UI thread. Live
sessions receive globally unique transient portal IDs, honor each node's frame
rate limit, synchronize directly connected canvas agents before observation,
and close when the node, floor, workspace, or application goes away. Undo and
imports restore only the durable configuration.

A real Chromium smoke test remains pending because no compatible executable is
installed on the development machine.

Coordinate clicks and wheel events now use the exact rendered-frame transform,
including canvas zoom and letterboxing. Clicking a frame gives it text focus;
ordinary text and IME commits are forwarded to the focused browser element.
Frame capture and local input share one serialized queue so a capture cannot
invalidate an action before dispatch. Enter, Shift-Tab, Backspace, Delete,
Escape, arrow keys, Home/End, and Page Up/Down use typed CDP key events. Browser
clipboard shortcuts remain separate because clipboard access has its own policy.

Device discovery now probes Appium, ADB, and Xcode independently. Recorded
fixtures cover Android emulators and attached-device authorization states plus
Apple simulators and attached devices. Missing host tools and unsupported Apple
hosts produce explicit unavailable reasons. Appium session creation and device
action execution remain to be implemented.
