import { frozenCopy, nonblank, object } from "./validation.mjs";

const events = new Set(["turn_started", "before_model", "before_tool", "after_tool", "before_stop", "turn_settled"]);

function responseFor(event, raw) {
  const response = raw === undefined ? { action: "continue" } : raw;
  const fields = {
    continue: [], model_context: ["messages", "instructions"],
    block_tool: ["reason"], tool_output: ["output"], tool_arguments: ["args"], continue_turn: ["reason"],
  }[response?.action];
  if (!fields) throw new Error("unsupported hook action");
  object(response, ["action", ...fields], [], "hook response");
  const permitted = { model_context: "before_model", block_tool: "before_tool", tool_output: "after_tool", tool_arguments: "before_tool", continue_turn: "before_stop" };
  if (response.action !== "continue" && permitted[response.action] !== event.event) {
    throw new Error(`action ${response.action} is not allowed for ${event.event}`);
  }
  if (["block_tool", "continue_turn"].includes(response.action)) nonblank(response.reason, "block reason");
  if (response.action === "tool_output" && typeof response.output !== "string") {
    throw new Error("tool output must be a string");
  }
  if (response.action === "model_context" &&
      (!Array.isArray(response.messages) || !Array.isArray(response.instructions))) {
    throw new Error("model context requires messages and instructions arrays");
  }
  return structuredClone(response);
}

/** Registration order composes contributions inside this one hook export. */
export function createHooks(settings = {}) {
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) {
    throw new Error("hook settings must be an object");
  }
  const registrations = [];
  let sealed = false;
  const api = Object.freeze({
    config: frozenCopy(settings),
    on(event, handler, options = {}) {
      if (sealed) throw new Error("register hooks only during setup");
      if (!events.has(event)) throw new Error(`unsupported hook event: ${event}`);
      if (typeof handler !== "function") throw new Error("hook handler must be a function");
      object(options, [], ["tools"], "hook options");
      if (options.tools !== undefined &&
          (!["before_tool", "after_tool"].includes(event) || !Array.isArray(options.tools) ||
           options.tools.length === 0 || options.tools.some((name) => typeof name !== "string" || !name.trim()))) {
        throw new Error("tools must be a nonempty list of exact tool names on a tool event");
      }
      registrations.push({ event, handler, tools: options.tools && new Set(options.tools) });
    },
  });
  return {
    api,
    seal() { sealed = true; },
    async invoke(input, signal) {
      object(input, ["event", "attribution", "cwd"], [], "HookInput");
      if (!events.has(input.event?.event)) throw new Error("unsupported canonical hook event");
      nonblank(input.cwd, "cwd");
      let event = structuredClone(input.event);
      let final = { action: "continue" };
      const ctx = Object.freeze({
        cwd: input.cwd, attribution: frozenCopy(input.attribution),
        config: api.config, signal,
      });
      for (const registration of registrations) {
        if (registration.event !== event.event ||
            (registration.tools && !registration.tools.has(event.call.name))) continue;
        signal.throwIfAborted();
        const result = await registration.handler(frozenCopy(event), ctx);
        signal.throwIfAborted();
        const response = responseFor(event, result);
        if (response.action === "continue") continue;
        final = response;
        if (["block_tool", "continue_turn"].includes(response.action)) break;
        if (response.action === "tool_arguments") { event.call.args = response.args; event.call.raw_arguments = null; }
        if (response.action === "tool_output") event.result.output = response.output;
        if (response.action === "model_context") {
          event.request.messages = response.messages;
          event.request.instructions = response.instructions;
        }
      }
      // The host validates canonical model identities/capabilities after this export.
      return { result: final };
    },
  };
}
