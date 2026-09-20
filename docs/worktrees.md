# Git worktree floors

Issue: https://github.com/Dnreikronos/openpodium/issues/15

## Behavior

Creation is explicit (confirmed for #15). A floor represents a Git checkout within a workspace;
it has its own canvas and may belong to an agent or task. The original checkout
is the main floor. Switching floors preserves running processes and canvas nodes.
Agents start in their node's checkout, independently of the selected floor.

Discover the repository from the workspace directory and inventory existing
worktrees through the Git CLI. Discovered checkouts remain user-owned. Only a
checkout successfully created and durably recorded by OpenPodium is managed.
Never infer ownership from a path or branch naming convention.

Validate branch and floor names before invoking Git. Use argument arrays, never
a shell command assembled from user input. Record the repository identity,
checkout path, branch, starting revision, optional owner, and lifecycle.
Dirty state is refreshed from Git, including untracked files.

Persist floor metadata, node membership, and active selection through the existing
domain journal and snapshots. Old workspaces restore with their original canvas
on the main floor. Missing checkouts remain visible and cannot launch agents.

Cleanup is explicit and limited to managed checkouts. Refuse dirty, locked, or
unmerged work by default, and refuse cleanup while its agents are running.
An explicit discard decision may authorize dirty or unmerged work removal;
the UI must identify the target and consequences. Existing user worktrees and
the original checkout can never be removed by this feature. Do not implicitly
merge or delete branches.

## Verification

Use temporary real Git repositories to test validation, discovery from nested
directories, creation, ownership, dirty/untracked files, unmerged commits, and
cleanup refusal. Test independent canvas layouts and restart restoration through
the existing SQLite journal. Run the compiler, formatter, Clippy, and targeted
tests after each implementation phase.

## Implementation notes

Use **Discover / refresh worktrees** in workspace settings to inventory existing
checkouts and refresh dirty/missing state. **Create floor** requires a floor name
and a new branch name. Selecting an agent or task node before creation records
that entity as the owner; existing processes are not moved. Add local agents on
the new floor to run them in its checkout. Remote environment profiles are not
supported on isolated floors in this iteration.

Managed directories live beside the application database under
`worktrees/<workspace-id>/<floor-name>`. A random ownership token is stored in
both the journal and the checkout's private Git metadata. Replacing a checkout,
changing its branch, or losing its ownership marker revokes cleanup authority.
Checkouts left behind by an interrupted creation are discovered as user-owned.

Git operations run on a blocking worker. Results are applied through the main
workspace manager, preserving canvas edits made while Git runs. Terminal launches
are blocked during Git operations; existing terminals keep running. Floor status
is refreshed explicitly rather than continuously polling every checkout.

Cleanup checks whether the checkout HEAD is an ancestor of the branch from which
it was created. Detached-base floors require manual integration and cleanup in
Git. Branches remain after checkout removal, and removed floors retain their
canvas history. Pan/zoom, selection, and undo history reset when switching floors,
matching existing workspace switching; node geometry and floor selection persist.
