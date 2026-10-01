import { runCommand } from "../command.mjs";
import { stopDecision } from "../ports.mjs";

export default function setup(hooks) {
  const { command, args = [] } = hooks.config;
  hooks.on("before_stop", async (event, ctx) => stopDecision(await runCommand(command, args, {
    hook_event_name: "Stop", cwd: ctx.cwd,
    session_id: ctx.attribution.agent?.session_id ?? null,
    turn_id: ctx.attribution.agent?.turn_id ?? null,
    stop_hook_active: event.attempt > 0,
    last_assistant_message: event.output.text,
  }, ctx)));
}
