# Canvas and terminal responsiveness

The September 2026 debug-build profile showed repeated terminal grid conversion
during app views and per-cell background tessellation during canvas redraws.
Panning changes the camera, not terminal contents, so it should not rebuild the
terminal's strings. Typing should update the affected session without copying
every other terminal's cells.

Each session now lazily caches its derived view. Cell data is immutable and shared
across view clones. Output, process status, resizing, scrolling, and selection
invalidate the cache; unchanged dimensions and transcript persistence do not.
The terminal model remains the source of truth. Theme colors are resolved while
drawing, so cached cells still follow appearance changes.

Terminal backgrounds reuse the already-painted card surface. Other backgrounds
are merged into adjacent same-color runs within a row. Runs never bridge missing
cells or rows; explicit colors, inverse video, and selection retain their original
coverage. Text clipping and node stacking are unchanged. Label construction no
longer converts terminal contents into a semantic value that it discards.

Regression checks cover shared cell allocation, cache invalidation, snapshot
round trips, labels, and equivalence between batched and per-cell backgrounds.
For an 80-column, 30-row screen, background rectangle calls fall from 2,400 to
zero for default cells or 30 for a uniform explicit background. These are draw
counts, not frame-rate or end-to-end latency measurements.

Validate an optimized release build with a populated terminal: pan the board,
type into a running shell, scroll history, select text, and switch appearance.
Keep profiling results separate from responsiveness observations, and do not
install a debug build as the normal macOS application.

## Workspaces with many Git worktrees

Profiling the affected `supa-skeleton` workspace exposed a separate blocker:
987 of 996 main-thread samples were inside `node_severities`, calling
`CollisionReport::severity_for` and repeatedly canonicalizing checkout paths for
every collision. This work ran during view construction, so terminal caching did
not address it.

The background Git scan now builds an immutable index of checkout identities,
maximum collision severity, and inventory positions. Rendering and changed-path
lookups only read that index; they perform no filesystem calls and never iterate
the collision list. Workspace and floor path aliases are resolved in the worker
and included in the scan signature so newly added aliases are indexed even when
Git contents are unchanged. Existing scan invalidation still rejects results
after the workspace directory changes.

Regression coverage includes 10,001 collisions resolving only three distinct
paths, maximum-severity aggregation, symlink aliases, and inventory lookup after
the alias has been removed. This keeps filesystem availability out of rendering.
Git scanning itself can still take time on repositories with many worktrees;
its completion must not block board interaction.

An isolated copy of the affected workspace was checked against its real Git
repository. After the full scan finished, the critical badges were present and
940 of 981 main-thread samples were waiting in the event loop. No render-time
severity/path-resolution stack appeared. This is sampled-thread evidence, not
an FPS measurement. The copied database and running terminals were separate from
the installed app.
