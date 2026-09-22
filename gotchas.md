# Gotchas

- A bounded terminal cache must store a self-contained screen, not an arbitrary tail of escape sequences. A newline does not reset SGR or terminal modes. Acknowledge cache writes only after storage succeeds, and check all floors before treating a cache entry as orphaned.
- A rendering budget must fail closed: omit excess visible fragments rather than fall back to painting a rectangle that includes occluded content.

- When a UI change exists only in a new build, lead the handoff with that limitation. Do not imply the control is already visible in the running app; explain that switching builds requires a restart and confirm before interrupting active terminals.

- Deleting an agent window must also remove its agent record, not merely hide it or adjust the sidebar count. Account for shared cards, saved conversations, task references, persistence, and undo before changing the deletion lifecycle.

- A connection tool must expose a source-to-target gesture with visible preview and cancellation. Silently disabling it until two cards are selected makes it look broken; verify interactions against the reference video, not just the final connected appearance.

- Close temporary QA app instances after verification and confirm only the user's existing instance remains. Do not leave duplicate OpenPodium windows running after a UI test.

- A modal's entire visible surface must capture pointer events, including labels, status messages, padding, and empty space. Only the backdrop outside the card may dismiss it. Scope editor feedback to the editing operation and fit dialog height to its contents.

- Removing a panel must preserve obvious entry points into the controls it contained. Notes need a body-click/Enter editing path, keyboard focus on the visible editor, and initialized content immediately after creation; a distant generic edit button is not enough.

- A canvas UI should not surface internal orchestration logs, agent registries, and configuration forms as a default inspector. When the user asks to remove that panel, remove the entry point and its dashboard; expose any retained essential action only where it is relevant.

- When the user asks to work on a well-scoped issue, proceed with implementation after inspection. Make reasonable assumptions and document them instead of stopping at a clarification question when the issue already supplies acceptance criteria.
- Windows filesystem canonicalization adds verbatim path prefixes that Git worktree commands may reject. Normalize paths consistently at the Git boundary for both arguments and identity comparisons, and test discovery, creation, ownership, and cleanup with canonicalized input paths on Windows CI.
- When CI failures depend on random fixtures or platform startup time, make invalid values differ deterministically and give heavyweight Windows subprocesses a Windows-specific timing budget.
- When a portable document contains relationships or references, test the export and import paths together on every supported platform. Keep secret checks shared across formats, allocate IDs across hidden floors, and model domain invariants such as retry predecessor state explicitly instead of dropping the relationship during import.
- Portal automation needs end-to-end production-path checks, not only backend tests. Keep visual refresh separate from semantic revision expiry, bind grants to the live target, expose desktop approval controls, validate full idempotency identity, and prove agents can retrieve frames and act on observed element references.
- A durable scheduler must record a dispatch before the work leaves the process, and must treat a dispatched step whose outcome is unknown as interrupted rather than failed. A cancelled handoff does not prove the agent stopped, so conflicting reservations stay held until a user confirms shutdown. Never infer completion from terminal text; extend the typed protocol when structured results are needed.
- Accessibility controls cannot be dropped into an already dense sidebar as raw default widgets. Treat visual hierarchy, spacing, grouping, and the empty state as part of the feature; validate the running app at its default window size before presenting it for review.
- Never use unverified global screen coordinates for native-app validation when other windows are underneath. Target and focus the OpenPodium process explicitly, capture its window directly, and avoid interacting with unrelated applications.
- Desktop project onboarding should use the operating system's native folder picker. Do not make users type or paste absolute filesystem paths when the platform already provides a safer, familiar selection flow.
- Core canvas navigation must stay visible and work while embedded content is focused. Route zoom shortcuts around terminal and portal input capture, and provide persistent zoom controls instead of relying on undiscoverable gestures or keys.
- Do not conflate trackpad scrolling with pinching: two-finger scroll pans the board, the native magnification gesture zooms, and a mouse wheel zooms. Frameworks may discard native gesture events, so verify the full platform event path instead of approximating pinch with pixel-scroll deltas.
