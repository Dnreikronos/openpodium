# Accessibility release checklist

Record the operating system, desktop environment, display scale, OpenPodium
revision, locale, input method, and assistive technology version with each run.

## Keyboard-only workflow

- Create or select a workspace without using a pointer.
- Add two agent nodes, select each node, and start and stop its terminal.
- Traverse nodes and connections in both directions and confirm wraparound.
- Open the command palette, search, choose a result, and return focus to the
  canvas.
- Create a task or handoff, inspect its status, and invoke its available
  recovery action.
- Reach every visible form control with `Tab` and `Shift+Tab`; focus is visible
  and follows reading order.
- Confirm focused terminals receive arrows, function keys, `Escape`, and
  ordinary editing shortcuts while documented global shortcuts still work.

## Screen reader

Run with VoiceOver on macOS, NVDA or Narrator on Windows, and Orca on Linux.

- The application and active workspace have useful names.
- Workspace controls announce their role, label, state, and shortcut.
- The canvas announces its node count and the selected node.
- Each canvas node announces its kind, name, status, position in the set, and
  available actions.
- Tasks announce owner and lifecycle status; agents announce runtime status.
- Activating, selecting, starting, stopping, and recovery actions affect the
  same objects as their visual controls.
- Status and error notices are announced once and do not steal focus.
- Terminal output is readable as bounded text and secret input is not exposed.

## Scaling, contrast, and motion

- At 100%, 150%, and 200% UI scale, essential controls neither clip nor
  overlap and remain reachable by scrolling.
- At 200% operating-system display scale, canvas and portal hit targets still
  align with their visuals.
- High-contrast mode preserves readable text, visible focus, selection, error,
  warning, and success states without relying on color alone.
- Reduced-motion mode contains no non-essential animated transitions.
- Canvas zoom does not reduce application chrome below the selected UI scale.

## International input and layout

- Enter composed characters with dead keys in every editable field.
- Enter and edit CJK text through an IME; pre-edit and committed text appear in
  the intended field or terminal.
- Enter emoji, combining marks, and supplementary-plane characters.
- Use a non-US keyboard layout and confirm shortcuts follow named/logical keys
  as documented rather than US physical key positions.
- With the validation RTL locale, headings, values, controls, and scrolling
  remain usable and user-provided bidirectional text is preserved.
- Locale-sensitive counts, numbers, and timestamps use the selected locale.

## Result

- [ ] macOS passed
- [ ] Windows passed
- [ ] Linux passed
- [ ] Any exceptions have linked issues and release notes
