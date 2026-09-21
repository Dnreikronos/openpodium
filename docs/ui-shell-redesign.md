# Application shell redesign

Reference: https://www.themaestri.app/en

## Direction

OpenPodium is a canvas application. The canvas or empty-state invitation must
own the window, while navigation and configuration stay visually secondary.
The reference is used for hierarchy and restraint, not copied branding or
assets.

The default shell uses a warm near-white canvas, dark text, hairline borders,
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
- With a workspace open, let the canvas take most of the window. Put detailed
  workspace controls in a right inspector with section cards and scrolling.
- Keep zoom out, current zoom/reset, and zoom in controls visible in the canvas
  toolbar. The same commands must work while a terminal or portal has focus.
- Make the mouse wheel zoom the canvas, two-finger trackpad scrolling pan the
  board, and the native trackpad pinch gesture zoom. Option/Alt + scrolling
  passes movement to terminal or portal content under the pointer.
- Show status as compact text or badges rather than a large block of controls.

## Components

- Primary actions use a dark or blue filled pill; secondary actions use quiet
  neutral fills or hairline borders.
- Section labels are small and muted. Headings use stronger size and weight,
  with more space above than below.
- Project selection uses the native folder picker instead of exposing a raw
  filesystem-path input.
- Accessibility preferences remain reachable but become compact footer
  controls instead of dominating the sidebar.
- High-contrast mode remains a separate black, white, cyan, yellow, green, and
  red palette.

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
