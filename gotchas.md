# Gotchas

- When the user asks to work on a well-scoped issue, proceed with implementation after inspection. Make reasonable assumptions and document them instead of stopping at a clarification question when the issue already supplies acceptance criteria.
- Windows filesystem canonicalization adds verbatim path prefixes that Git worktree commands may reject. Normalize paths consistently at the Git boundary for both arguments and identity comparisons, and test discovery, creation, ownership, and cleanup with canonicalized input paths on Windows CI.
- When CI failures depend on random fixtures or platform startup time, make invalid values differ deterministically and give heavyweight Windows subprocesses a Windows-specific timing budget.
