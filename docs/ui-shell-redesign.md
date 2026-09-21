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
  headline, one sentence, one path field, and one clear primary action.
- With a workspace open, let the canvas take most of the window. Put detailed
  workspace controls in a right inspector with section cards and scrolling.
- Show status as compact text or badges rather than a large block of controls.

## Components

- Primary actions use a dark or blue filled pill; secondary actions use quiet
  neutral fills or hairline borders.
- Section labels are small and muted. Headings use stronger size and weight,
  with more space above than below.
- Inputs use light filled surfaces, subtle borders, and consistent height.
- Accessibility preferences remain reachable but become compact footer
  controls instead of dominating the sidebar.
- High-contrast mode remains a separate black, white, cyan, yellow, green, and
  red palette.

## Acceptance

- At startup, workspace creation is the clear focal point.
- The sidebar no longer competes with the canvas or empty state.
- Primary and secondary actions are distinguishable without reading every
  label.
- The app remains usable at 100%, 150%, and 200% interface scale.
- Existing commands, persistence, canvas behavior, and accessibility settings
  keep working.
