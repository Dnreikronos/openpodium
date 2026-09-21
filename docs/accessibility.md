# Accessibility, localization, and international input

Issue: https://github.com/Dnreikronos/openpodium/issues/26

## Product contract

OpenPodium must expose the same workspace and orchestration state through
visual, keyboard, and assistive-technology interfaces. Accessibility metadata
is a projection of durable workspace state; it never becomes a second source
of truth. A native accessibility adapter must dispatch the same application
messages as pointer and keyboard actions.

The supported locales are US English and Brazilian Portuguese. The
localization boundary keeps essential application-shell strings behind stable
message identifiers, applies locale plural rules and number formatting, and
accepts an explicit locale override for deterministic tests. Unsupported
locales fall back to US English. A pseudo-localized right-to-left catalog is
used in validation to catch clipped, unmirrored, or accidentally hard-coded
content without presenting an unfinished translation to users.

## Semantic canvas

The visual canvas remains GPU-rendered, while an independent semantic snapshot
describes its current contents. Its root identifies the active workspace and
contains nodes in the same deterministic `(y, x, id)` order used by keyboard
navigation. Each node exposes:

- a stable identifier, role, name, kind, selection state, and position in the
  ordered set;
- status and value text for agents, tasks, terminals, and portals;
- the actions currently available for that node, including selection,
  activation, and context-specific start or stop actions;
- terminal text as a bounded, read-only value derived from the emulator
  snapshot, never from a duplicate terminal buffer.

Screen-reader actions resolve the semantic identifier back to the current
workspace and node before dispatch. Stale identifiers are ignored safely.

## Keyboard and focus

Application focus has three explicit regions: navigation, canvas, and active
content. `Tab` and `Shift+Tab` advance through application controls when text
or terminal input does not own the event. Existing command shortcuts remain
the canonical route for canvas traversal, selection, activation, editing, and
zoom. A focused terminal or portal receives ordinary text, dead-key, IME, and
alternative-layout input; only documented global commands bypass it.

Focus order follows visual reading order. No operation that is required for a
primary workflow may be pointer-only. Focus changes and semantic selection
must refer to the same active node without duplicating durable selection state.

## Presentation preferences

The application supports normal and high-contrast themes, a UI scale from 100%
through 200%, and reduced motion. The scale is applied to rendering and input
coordinates together, including canvas and terminal content. Controls use
wrapping or scrolling instead of clipping at the maximum supported scale.
OpenPodium currently has no non-essential animated transitions; polling and
live portal refresh are functional updates and remain active in reduced-motion
mode.

Preferences are local application settings. System preferences are used when
the platform exposes them, and explicit user choices take precedence.

## Toolkit boundary

Upstream Iced 0.14 does not yet publish a native accessibility tree. A
compatibility spike against the COSMIC Iced fork was rejected because its
moving WGPU, Winit, and AccessKit dependencies do not build reproducibly with
OpenPodium's pinned Rust and platform matrix. OpenPodium keeps a
toolkit-independent semantic snapshot and action model so the application can
adopt upstream support, or a separately proven adapter, without changing its
domain or input behavior.

Until that adapter exists, automated validation covers semantic completeness
and action routing but cannot satisfy the native screen-reader release gate.
The limitation is explicit: a release must not claim that gate has passed based
only on semantic-model tests. A future adapter update requires the full
cross-platform quality matrix and a manual assistive-technology smoke test.

## Validation

Automated checks cover:

- stable semantic ordering, roles, names, values, statuses, and actions;
- stale accessibility actions and unavailable actions;
- catalog lookup, plural selection, locale fallback, number formatting, and
  pseudo-localized RTL output;
- focus traversal and suppression while terminal or portal text input owns the
  event;
- text scaling bounds and high-contrast palette contrast;
- Unicode text, combining marks, dead-key commits, IME pre-edit and commit,
  right-to-left text, and physical-key-independent shortcuts.

Release validation additionally runs the checklist in
`docs/accessibility-checklist.md` with VoiceOver on macOS, NVDA or Narrator on
Windows, and Orca on Linux. Automated semantic checks are required in CI;
manual screen-reader results are recorded before a release is tagged.
