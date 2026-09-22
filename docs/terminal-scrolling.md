# Terminal scrolling

Wheel and two-finger trackpad scrolling over an agent window scroll that
terminal, without requiring focus or Option. Capture even fractional and
horizontal-only deltas so they never move the board. Accumulate small vertical
pixel deltas per hovered terminal into whole scrollback lines. Forward wheel
events to terminal applications that enable mouse reporting.

Scrolling outside terminal windows retains the existing board pan/zoom behavior.
Pinch zoom and window dragging are unchanged. Test the actual canvas event path
for fractional deltas, direction, mouse reporting, zoomed windows, and background
navigation; do not restart live agent sessions without confirmation.

Restoring a terminal must retain a bounded, styled scrollback in addition to its
last screen. Existing screen-only snapshots remain readable, but cannot recover
lines that were never saved. Verify the same earlier lines before and after a
snapshot round trip, including Unicode and colors. Offline terminals scroll
locally instead of sending mouse reports to a process that no longer exists.
