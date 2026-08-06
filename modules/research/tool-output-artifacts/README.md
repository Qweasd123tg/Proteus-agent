# tool-output-artifacts

Draft plugin pack for future tool-result post-processing.

This crate intentionally does not export a dylib `PluginRoot` yet. The current
plugin ABI has no `ToolResultProcessor` / `ToolOutputStore` slot, so wiring this
into core would make one storage strategy a runtime default. Keep it as an
`rlib` draft until the contract exists.

Intended behavior:

- receive a `ToolResult` after tool execution and before model feedback;
- save oversized `output` / `error` text into workspace artifacts;
- return a shortened preview with artifact paths in metadata;
- keep paths inside the workspace and reject symlink directory escapes.

Future slot sketch:

```text
ToolResultProcessor::process(input) -> ToolResult

input:
  cwd
  tool_name
  tool_result
  max_preview_bytes

module ids:
  none
  artifact_files
  session_artifacts
  compressed_artifacts
```
