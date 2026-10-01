import { piToolCall, piToolResult } from "../ports.mjs";
import type { Hooks } from "../hooks.d.mts";

export default function setup(hooks: Hooks) {
  // The handler body keeps Pi's toolName/input and block/reason shape.
  hooks.on("before_tool", piToolCall(async (event) => {
    if (event.toolName === "apply_patch") return { block: true, reason: "Изменение файлов запрещено этим hook." };
  }));
  hooks.on("after_tool", piToolResult(async (event) => ({
    content: [{ type: "text", text: event.content.map((part) => part.text).join("\n").slice(0, 1200) }],
  })), { tools: ["read_file"] });
}
