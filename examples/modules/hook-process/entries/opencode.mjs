import { openCodeToolAfter } from "../ports.mjs";

export default function setup(hooks) {
  hooks.on("after_tool", openCodeToolAfter(async (input, output) => {
    // A ported tool.execute.after body can retain its input/output shape.
    output.output = `${output.output}\nПроверено hook.`;
  }), { tools: ["read_file"] });
}
