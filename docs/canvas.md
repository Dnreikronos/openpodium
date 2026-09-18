# Canvas camera and rendering

The canvas keeps node geometry in world coordinates and applies camera transforms only when
rendering or hit-testing. The camera position is the world coordinate at the center of the
viewport. Its zoom is clamped to 25%–400%.

## Controls

- Drag the background with the left or middle mouse button to pan.
- Scroll a mouse wheel or use two-finger trackpad scrolling to pan.
- Hold the platform command key (`Cmd` on macOS, `Ctrl` elsewhere) while scrolling to zoom around
  the pointer.
- Use the arrow keys to pan while the pointer is over the canvas.
- Use `+` and `-` to zoom around the viewport center and `0` to reset the camera.

## Rendering contract

Iced's canvas widget submits cached geometry through its WGPU renderer. Camera changes invalidate
that geometry; unrelated events reuse it. Before any node geometry or text is generated, node
world bounds are intersected with the camera's visible world rectangle. The grid increases its
world spacing at lower zoom levels to avoid excessive draw work.

All transform calculations use 64-bit world coordinates. Conversion to Iced's 32-bit screen
coordinates happens only at the rendering boundary. Panning and zooming never mutate node world
coordinates, and pointer-anchored zoom preserves the world coordinate below the pointer.
