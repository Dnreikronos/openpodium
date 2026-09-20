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
existing canvas scene and route pointer, scroll, keyboard, resize, z-order, and
camera events through the phase 1 geometry.

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
