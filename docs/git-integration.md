# Git collisions and integration

Issue: https://github.com/Dnreikronos/openpodium/issues/16

## Changed paths

OpenPodium inventories each available checkout against the revision where its
work diverged. The inventory combines committed, staged, unstaged, and
non-ignored untracked changes. Git rename detection supplies both sides of a
rename; deletions remain in the inventory so deleting a file can collide with
another floor editing or renaming it. Ignored files are omitted. A generated
file is treated like any other path: it appears when tracked or non-ignored and
does not appear when ignored.

Refreshes run outside the UI thread. The application periodically checks Git's
cheap status signature and schedules a full inventory when it changes. Manual
refresh remains available. A failed refresh keeps the previous report visible
and shows the error instead of clearing collision warnings.

## Collision levels

Two active floors collide when their inventories touch the same current or
pre-rename path. An overlap is a warning. It becomes critical when a three-way
merge of the two committed heads reports an unmerged entry. Dirty changes may
still turn a warning into a real conflict, so they are never labeled safe.

Floor warnings are projected onto every agent or task node assigned to that
floor. The settings panel shows the paths and the other floors involved.
Collision reports are derived state and are rebuilt after restart.

## Preview

Integration uses a source floor and the original checkout as its target. A
preview records both heads, source-only commits, the diff summary, dirty state,
and likely conflicts. Conflict prediction uses a temporary Git index and does
not touch either checkout. A preview cannot execute after either head or dirty
state changes.

The source and target must be available checkouts from the same repository.
Merge and cherry-pick require a clean target. Rebase requires a clean source.
The preview explains any condition that blocks an action.

## Actions

The user chooses one action from the preview:

- Merge applies the source head to the target with Git's normal merge command.
- Cherry-pick applies the source-only commits to the target in chronological
  order.
- Rebase rebases the source onto the target head. It does not update the target;
  the user previews again before the final merge or fast-forward.
- Leave alone closes the preview without invoking Git.

OpenPodium never aborts a failed integration automatically. Git's merge,
cherry-pick, or rebase state remains available for recovery. The error names
the affected checkout and gives the matching continue or abort commands. A new
preview is required after the user resolves or aborts the operation.

## Verification

Tests use temporary repositories and cover committed and uncommitted changes,
ignored and generated files, renames, deletions, overlap severity, stale
previews, every integration choice, and conflict recovery. Compiler, formatter,
Clippy, and targeted Git tests run before publication.
