The selected `direct` patch module uses this format for the `apply_patch` tool:
`*** Begin Patch`, one or more file operations, and `*** End Patch`.
Send the text in the function's string `patch` argument.

- `*** Add File: relative/path` is followed by `+`-prefixed lines and requires a new file.
- `*** Delete File: relative/path` deletes an existing file.
- `*** Update File: relative/path` accepts optional `*** Move to: new/path`, then chunks starting with bare `@@`. Chunk lines start with a space for exact context, `-` for removal, or `+` for addition.
- This module does not accept `@@ context`, positional unified diff headers, or `diff --git`. Its `*** End of File` marker removes the final newline.
- All operations are verified before writing; a failed commit rolls back earlier writes. Paths must be relative to the workspace and must not traverse symlinks.

Example: `*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old line\n+new line\n*** End Patch`.
