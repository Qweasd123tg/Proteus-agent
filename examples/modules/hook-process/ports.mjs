import { isDeepStrictEqual } from "node:util";
import { frozenCopy, nonblank, object } from "./validation.mjs";

// These explicit porting helpers expose a bounded subset of upstream payloads.
// They do not load upstream extensions or emulate an upstream runtime.
export function piToolCall(handler) {
  return async ({ call }, ctx) => {
    const event = Object.freeze({
      type: "tool_call", toolCallId: call.id, toolName: call.name, input: structuredClone(call.args),
    });
    const result = await handler(event, Object.freeze({ ...ctx, hasUI: false }));
    if (result === undefined) return !isDeepStrictEqual(event.input, call.args)
      ? { action: "tool_arguments", args: event.input } : undefined;
    object(result, [], ["block", "reason"], "ported Pi tool_call result");
    if (result.block === true) return { action: "block_tool", reason: nonblank(result.reason, "block reason") };
    if (result.block !== undefined && result.block !== false) throw new Error("block must be boolean");
    if (result.reason !== undefined) throw new Error("reason requires block: true");
    if (!isDeepStrictEqual(event.input, call.args)) return { action: "tool_arguments", args: event.input };
  };
}

export function piToolResult(handler) {
  return async ({ call, result }, ctx) => {
    if (result.content.length) throw new Error("Pi text-output port requires an unstructured tool result");
    const content = [{ type: "text", text: result.output }];
    const details = result.metadata;
    const isError = !result.ok;
    const answer = await handler(frozenCopy({
      type: "tool_result", toolCallId: call.id, toolName: call.name,
      input: call.args, content, details, isError,
    }), Object.freeze({ ...ctx, hasUI: false }));
    if (answer === undefined) return;
    object(answer, [], ["content", "details", "isError"], "ported Pi tool_result result");
    if ((Object.hasOwn(answer, "details") && !isDeepStrictEqual(answer.details, details)) ||
        (Object.hasOwn(answer, "isError") && answer.isError !== isError)) {
      throw new Error("hook/v2 cannot change tool status or metadata");
    }
    if (answer.content === undefined) return;
    if (!Array.isArray(answer.content)) throw new Error("content must be an array");
    for (const part of answer.content) {
      object(part, ["type", "text"], [], "ported text content");
      if (part.type !== "text" || typeof part.text !== "string") throw new Error("only text content can be ported");
    }
    return { action: "tool_output", output: answer.content.map((part) => part.text).join("\n") };
  };
}

function openCodeInput(call, ctx) {
  return frozenCopy({ tool: call.name, callID: call.id, sessionID: ctx.attribution.agent?.session_id ?? null });
}

export function openCodeToolBefore(handler) {
  return async ({ call }, ctx) => {
    const output = { args: structuredClone(call.args) };
    const result = await handler(openCodeInput(call, ctx), output);
    if (result !== undefined) throw new Error("ported OpenCode handlers must mutate output and return void");
    object(output, ["args"], [], "ported OpenCode before output");
    if (!isDeepStrictEqual(output.args, call.args)) return { action: "tool_arguments", args: output.args };
    // Handler exceptions remain explicit hook failures; they are not converted to a veto.
  };
}

export function openCodeToolAfter(handler) {
  return async ({ call, result }, ctx) => {
    const output = { output: result.output };
    const answer = await handler(frozenCopy({ ...openCodeInput(call, ctx), args: call.args }), output);
    if (answer !== undefined) throw new Error("ported OpenCode handlers must mutate output and return void");
    object(output, ["output"], [], "ported OpenCode after output");
    if (typeof output.output !== "string") throw new Error("tool output must be a string");
    if (output.output !== result.output) return { action: "tool_output", output: output.output };
  };
}

/** Convert the shared PreToolUse deny/allow + updatedInput subset. */
export function preToolUseDecision({ code, stdout, stderr }) {
  if (code === 2) return { action: "block_tool", reason: nonblank(stderr.trim(), "exit 2 reason") };
  if (code !== 0) throw new Error(`command hook exited ${code}: ${stderr.trim()}`);
  if (!stdout.trim()) return;
  const output = object(JSON.parse(stdout), [], ["hookSpecificOutput"], "ported PreToolUse output");
  if (!output.hookSpecificOutput) return;
  const decision = object(output.hookSpecificOutput, ["hookEventName", "permissionDecision"],
    ["permissionDecisionReason", "updatedInput"], "ported PreToolUse decision");
  if (decision.hookEventName !== "PreToolUse") throw new Error("expected PreToolUse decision");
  if (decision.permissionDecision === "deny") {
    if (Object.hasOwn(decision, "updatedInput")) throw new Error("updatedInput requires allow");
    return { action: "block_tool", reason: nonblank(decision.permissionDecisionReason, "deny reason") };
  }
  if (decision.permissionDecision !== "allow") throw new Error("only deny/allow decisions can be ported");
  if (Object.hasOwn(decision, "updatedInput")) return { action: "tool_arguments", args: decision.updatedInput };
  // "allow" continues through Proteus policy/approval; it never grants permissions.
}

/** Shared Stop block decision: request another attempt in the same Proteus turn. */
export function stopDecision({ code, stdout, stderr }) {
  if (code === 2) return { action: "continue_turn", reason: nonblank(stderr.trim(), "exit 2 reason") };
  if (code !== 0) throw new Error(`command hook exited ${code}: ${stderr.trim()}`);
  if (!stdout.trim()) return;
  const output = object(JSON.parse(stdout), [], ["decision", "reason"], "ported Stop output");
  if (output.decision === undefined && output.reason === undefined) return;
  if (output.decision !== "block") throw new Error("only the Stop block decision can be ported");
  return { action: "continue_turn", reason: nonblank(output.reason, "stop reason") };
}
