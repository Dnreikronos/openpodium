# Canvas connections

Reference: https://cdn.maestri.dev/assets/demo.mp4, around 20–22 seconds.

The link tool must work without preselecting two cards. With one card selected,
it starts a connection from that card; otherwise the user picks the source.
The pointer carries a dashed preview to the destination. Clicking the second
card, or dragging from source to destination, creates one durable connection.
The completed curve attaches to card edges and follows moved or resized cards.

Connection mode has a visibly active tool and short on-canvas instructions.
It intercepts node input so notes do not open and terminals do not receive
typing while the user chooses endpoints. Escape, the tool toggle, or clicking
empty canvas cancels without modifying the workspace. Invalid/self/duplicate
links do not persist, and show feedback beside the tool. Existing directional
connection types, undo/redo, and workspace persistence remain authoritative.
Selecting two cards first still supports connecting them in one click.

Verify creation, cancellation, invalid targets, dragging, zoom-aware hit tests,
edge geometry, persistence, and undo/redo with targeted tests and a temporary
UI instance. Close the temporary instance after verification.
