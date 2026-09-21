# Application shell redesign

Reference: https://www.themaestri.app/en

## Inspector removal (2026-09-21)

The workspace inspector and its toolbar toggle are removed. The canvas should
not compete with an agent registry, event log, orchestration dashboard, or
stack of unrelated configuration forms. Selecting a workspace or node must
not reopen that dashboard. Workspace order, canvas navigation, and existing
workspace data remain intact.

Keep note editing and terminal actions contextual to the selected node. Open
workspace settings from the workspace location menu, with less common tools
behind a separate menu entry. These focused dialogs never reduce canvas width.

## Demo alignment (2026-09-21)

Visual reference: https://cdn.maestri.dev/assets/demo.mp4

Match the demo's visual style and layout across the application while keeping
the existing workflows. Use a 212-pixel workspace rail, compact neutral search
and selection rows, and a canvas with a small outer inset. Keep workspace rows
in creation order when selection changes. Replace the text-heavy canvas tool
strip with consistent line icons and hover labels; every action must continue
to dispatch its existing command. Move the workspace path into a compact
bottom-left location label so it does not compete with the creation tools.

Use cool near-white surfaces, restrained shadows, small corner radii, and
lighter dialog sections. Reduce the visual weight of the canvas grid and
node headers, with a compact colored kind marker and the existing dashed blue
selection outline. Keep terminal sizing and pointer coordinates tied to the
same header-height constant. Preserve high-contrast colors and display
preferences, and verify the shell at narrow and standard window widths.

Implementation passes: shell and controls first, then canvas and node styling.
Run formatting, compilation, Clippy, and targeted canvas, terminal, navigation,
and presentation checks. Inspect the rendered application before handing off.

## Direction

OpenPodium is a canvas application. The canvas or empty-state invitation must
own the window, while navigation and configuration stay visually secondary.
The reference is used for hierarchy and restraint, not copied branding or
assets.

The default shell uses a cool near-white canvas, dark text, hairline borders,
soft neutral cards, one blue action color, and rounded surfaces. Dense dark
panels and browser-like stacks of full-width default buttons are out of scope
for the primary view.

## Layout

- Open at 1280 by 820 pixels, with a useful minimum size.
- Keep the workspace rail narrow and calm: brand, command search, workspace
  list, then a small footer for display preferences.
- Move workspace creation into the main empty state. The first-run view has one
  headline, one sentence, and one clear action that opens the operating
  system's native folder picker.
- With a workspace open, let the canvas take most of the window. Open detailed
  workspace controls in focused dialogs from the workspace menu.
- Float the canvas controls over the board rather than stacking them above it,
  so no control steals height from the work surface. Three persistent clusters: the
  workspace location bottom left, node and agent creation top center, the
  zoom bottom right. Selected-node actions appear above the footer. Each is a rounded pill on
  the canvas sheet. A pill only captures the pointer where a control sits, so
  panning and zooming still work across the rest of the board.
- Keep zoom out, current zoom/reset, and zoom in controls visible in the canvas
  toolbar. The same commands must work while a terminal or portal has focus.
- Open the command palette as a centered modal over a dimmed scrim. It must not
  push the application down the window.
- Make the mouse wheel zoom the canvas, two-finger trackpad scrolling pan the
  board, and the native trackpad pinch gesture zoom. Option/Alt + scrolling
  passes movement to terminal or portal content under the pointer.
- Show status as compact text or badges rather than a large block of controls.

## Components

Surfaces, controls, and layout helpers live in `src/app/shell.rs` and
`src/app/ui.rs`. Panels build from those instead of restating styling, so the
canvas renderer, dialogs, and the palette stay in step.

- Three greys carry the whole light theme: chrome behind the rail and the
  canvas backdrop, white for raised surfaces, and one recessed neutral for
  tracks, badges, key caps, and hover fills. Iced's derived `weak` neutral is
  too saturated to sit behind a window, so those values are explicit.
- Primary actions use a dark or blue filled pill; secondary actions use quiet
  neutral fills or hairline borders. Destructive actions stay quiet until
  hovered, then turn red.
- Section labels are small and muted. Headings use stronger size and weight,
  with more space above than below.
- Related controls group into titled section cards. Clusters of equal-weight
  actions lay out in a grid that reflows to the panel width, so a narrow
  dialog never clips a button.
- Lists select by highlighting the row, not by prefixing a check mark, and a
  row's own actions unfold only while it is selected.
- Mutually exclusive choices use a segmented picker rather than several
  lookalike buttons.
- Project selection uses the native folder picker instead of exposing a raw
  filesystem-path input.
- Accessibility preferences remain reachable but become compact footer
  controls instead of dominating the sidebar.
- High-contrast mode remains a separate black, white, cyan, yellow, green, and
  red palette. Secondary text barely fades there, because in that palette
  nothing may recede out of legibility.

## Canvas nodes

- A node is a white card with a hairline border, a compact header band closed by
  a hairline, and a small colored marker naming its kind. The accent is not the
  border, so an unselected board reads as one material.
- Selection is a dashed accent outline plus a small rounded grip in the corner,
  which reads as a marquee rather than a permanently heavier border.
- Connections are dashed curves with horizontal control points. A straight
  center-to-center line passes under the nodes it joins; a curve does not.
- The grid is orientation, not decoration: visible up close, never competing
  with the nodes drawn over it.
- Notes use a soft yellow surface in the default theme. Terminal defaults use
  a white background and dark foreground, with ANSI colors chosen for that
  background. Explicit application RGB colors remain intact, and background
  queries report the light default so terminal programs can adapt their themes.
- Changing contrast invalidates the cached canvas geometry immediately, so
  node surfaces and labels update together with the surrounding controls.

## Acceptance

- At startup, opening a project is the clear focal point and never requires
  manually entering a path.
- The sidebar no longer competes with the canvas or empty state.
- Primary and secondary actions are distinguishable without reading every
  label.
- The app remains usable at 100%, 150%, and 200% interface scale.
- Existing commands, persistence, canvas behavior, and accessibility settings
  keep working.
- Zoom controls remain usable over empty canvas space and focused agent,
  terminal, or portal nodes.
- Mouse-wheel zoom, trackpad pan, and trackpad pinch zoom work without a
  modifier key.
- Panning and zooming keep working over the canvas everywhere a floating pill
  is not directly under the pointer.
- There is no Inspector button, agent registry, or activity-log dashboard.
- Note editing, terminal controls, and workspace settings remain reachable
  without a permanent side panel or changes to the canvas viewport.
