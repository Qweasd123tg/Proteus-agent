import { setTimeout } from "node:timers/promises";
import { writeFile } from "node:fs/promises";
import { openCodeToolBefore } from "../../../../examples/modules/hook-process/ports.mjs";

export default function setup(hooks) {
  if (hooks.config.args !== undefined) {
    hooks.on("before_tool", openCodeToolBefore((input, output) => {
      output.args = structuredClone(hooks.config.args);
    }), { tools: ["read_file"] });
  }
  hooks.on("before_stop", async (event, ctx) => {
    if (ctx.config.marker) await writeFile(ctx.config.marker, "review entered");
    if (ctx.config.delay) await setTimeout(ctx.config.delay, undefined, { signal: ctx.signal });
    if (ctx.config.fail) throw new Error("review failed");
    if (event.attempt < (ctx.config.continuations ?? 0)) {
      return { action: "continue_turn", reason: "Проверь ответ ещё раз." };
    }
  });
}
