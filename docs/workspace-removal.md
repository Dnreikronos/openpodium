# Remove a workspace

Show a small trash action beside the selected workspace in the sidebar. Ask for
confirmation with the workspace name before removing it. Cancel and Escape must
leave the workspace unchanged. Removal stops only that workspace's terminals,
removes its runtime registrations and search results, and switches to another
workspace or the empty state.

Remove the workspace from the persistent open-workspace registry, not from disk.
Keep project files, notes, the journal and screen caches. Opening the same folder
again restores the saved workspace and its identity. Never reuse a removed
workspace ID. Update active selection and registry membership in one transaction;
a storage error must leave in-memory state unchanged.

Verify active/inactive/last-workspace removal, cancellation, reopen recovery,
stable IDs, preservation of files, failed-write rollback, and scoped runtime
cleanup. Reject late confirmation or search-index results for removed workspaces.
