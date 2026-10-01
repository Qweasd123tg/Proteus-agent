import { runCommand } from "../command.mjs";
import { preToolUseDecision } from "../ports.mjs";

export default function setup(hooks) {
  const { command, args = [] } = hooks.config;
  hooks.on("before_tool", async ({ call }, ctx) => {
    // Shared PreToolUse fields used by Codex and Claude command scripts.
    // Tool names/argument schemas remain Proteus names; map them here if needed.
    const input = {
      hook_event_name: "PreToolUse", cwd: ctx.cwd,
      session_id: ctx.attribution.agent?.session_id ?? null,
      tool_use_id: call.id, tool_name: call.name, tool_input: call.args,
    };
    return preToolUseDecision(await runCommand(command, args, input, ctx));
  });
}
