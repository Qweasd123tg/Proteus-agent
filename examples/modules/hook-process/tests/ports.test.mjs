import assert from "node:assert/strict";
import { test } from "node:test";
import { createHooks } from "../hooks.mjs";
import { piToolCall, piToolResult, openCodeToolBefore, openCodeToolAfter, preToolUseDecision, stopDecision } from "../ports.mjs";
import { runCommand } from "../command.mjs";

const attribution = { execution_id: "execution", agent: { session_id: "session", thread_id: "thread", turn_id: "turn" } };
const call = { id: "call", name: "read_file", args: { path: "file" }, surface: "function", raw_arguments: null };
const before = { event: "before_tool", call, spec: null, blocked: null };
const after = { event: "after_tool", call, result: { call_id: "call", ok: true, output: "original", content: [], error: null, metadata: null } };
const invoke = (hooks, event, signal = new AbortController().signal) => hooks.invoke({ event, attribution, cwd: process.cwd() }, signal);

test("Pi veto and exact tool filters preserve the handler body", async () => {
  const hooks = createHooks();
  let later = false;
  hooks.api.on("before_tool", piToolCall(async (event, ctx) => {
    assert.equal(ctx.hasUI, false);
    assert.equal(event.toolCallId, "call");
    if (event.input.path === "file") return { block: true, reason: "owner veto" };
  }), { tools: ["read_file"] });
  hooks.api.on("before_tool", () => { later = true; });
  assert.deepEqual((await invoke(hooks, before)).result, { action: "block_tool", reason: "owner veto" });
  assert.equal(later, false);
  assert.deepEqual((await invoke(hooks, { ...before, call: { ...call, name: "other" } })).result, { action: "continue" });
});

test("Pi and OpenCode output handlers compose in registration order", async () => {
  const hooks = createHooks();
  hooks.api.on("after_tool", piToolResult((event) => ({ content: [{ type: "text", text: event.content[0].text.toUpperCase() }] })));
  hooks.api.on("after_tool", openCodeToolAfter((input, output) => {
    assert.equal(input.sessionID, "session");
    assert.equal(input.args.path, "file");
    output.output += "!";
  }));
  assert.deepEqual((await invoke(hooks, after)).result, { action: "tool_output", output: "ORIGINAL!" });
  assert.equal(after.result.output, "original");
});

test("unsupported upstream behavior fails explicitly", async () => {
  const hooks = createHooks();
  assert.throws(() => hooks.api.on("session_start", () => {}), /unsupported hook event/);
  hooks.api.on("before_tool", openCodeToolBefore((input, output) => { output.args.path = "other"; }));
  assert.deepEqual((await invoke(hooks, before)).result, { action: "tool_arguments", args: { path: "other" } });
  const status = createHooks();
  status.api.on("after_tool", piToolResult(() => ({ isError: true })));
  await assert.rejects(invoke(status, after), /cannot change tool status/);
  const structured = createHooks();
  structured.api.on("after_tool", piToolResult(() => ({ content: [] })));
  await assert.rejects(invoke(structured, { ...after, result: { ...after.result, content: [{ type: "image" }] } }), /unstructured/);
  const wrongPhase = createHooks();
  wrongPhase.api.on("turn_settled", () => ({ action: "block_tool", reason: "continue" }));
  await assert.rejects(invoke(wrongPhase, { event: "turn_settled", status: "success", output: null, error: null }), /not allowed/);
});

test("canonical data are immutable and throw semantics remain explicit", async () => {
  const hooks = createHooks();
  hooks.api.on("before_tool", piToolCall((event) => { event.input.path = "other"; }));
  assert.deepEqual((await invoke(hooks, before)).result, { action: "tool_arguments", args: { path: "other" } });
  assert.equal(before.call.args.path, "file");
  const immutable = createHooks();
  immutable.api.on("before_tool", event => { event.call.name = "other"; });
  await assert.rejects(invoke(immutable, before), TypeError);
  const throwing = createHooks();
  throwing.api.on("before_tool", openCodeToolBefore(() => { throw new Error("ported failure"); }));
  await assert.rejects(invoke(throwing, before), /ported failure/);
});

test("Codex/Claude PreToolUse veto subset and unsupported decisions", () => {
  const denied = JSON.stringify({ hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: "owner veto" } });
  assert.deepEqual(preToolUseDecision({ code: 0, stdout: denied, stderr: "" }), { action: "block_tool", reason: "owner veto" });
  assert.deepEqual(preToolUseDecision({ code: 2, stdout: "ignored", stderr: "blocked" }), { action: "block_tool", reason: "blocked" });
  assert.equal(preToolUseDecision({ code: 0, stdout: "", stderr: "" }), undefined);
  for (const output of [
    { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", updatedInput: {} } },
    { continue: false, stopReason: "stop" },
    { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "ask" } },
  ]) assert.throws(() => preToolUseDecision({ code: 0, stdout: JSON.stringify(output), stderr: "" }));
  assert.throws(() => preToolUseDecision({ code: 1, stdout: "", stderr: "failed" }), /exited 1/);
});

test("command bridge sends JSON on stdin and waits for cancellation cleanup", async () => {
  const ctx = { cwd: process.cwd(), signal: new AbortController().signal };
  const result = await runCommand(process.execPath, ["--input-type=module", "-e",
    "let text='';for await(const chunk of process.stdin)text+=chunk;console.log(JSON.parse(text).value)"], { value: "payload" }, ctx);
  assert.equal(result.stdout, "payload\n");
  const controller = new AbortController();
  const pending = runCommand(process.execPath, ["-e", "setInterval(()=>{},1000)"], {}, { ...ctx, signal: controller.signal });
  setTimeout(() => controller.abort(new Error("owner cancel")), 50);
  await assert.rejects(pending, /owner cancel/);
});

test("argument rewrites compose and Stop review ends the ordered chain", async () => {
  const hooks = createHooks();
  hooks.api.on("before_tool", piToolCall(event => { event.input.path = "pi"; }));
  hooks.api.on("before_tool", openCodeToolBefore((input, output) => { output.args.path += ".ts"; }));
  assert.deepEqual((await invoke(hooks, before)).result, { action: "tool_arguments", args: { path: "pi.ts" } });
  const json = JSON.stringify({ hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "allow", updatedInput: { path: "codex" } } });
  assert.deepEqual(preToolUseDecision({ code: 0, stdout: json, stderr: "" }), { action: "tool_arguments", args: { path: "codex" } });
  hooks.api.on("before_stop", () => stopDecision({ code: 0, stdout: '{"decision":"block","reason":"check tests"}', stderr: "" }));
  hooks.api.on("before_stop", () => { throw new Error("must not run after continuation"); });
  assert.deepEqual((await invoke(hooks, { event: "before_stop", task: {}, history: [], output: {}, attempt: 0, continuation: null })).result,
    { action: "continue_turn", reason: "check tests" });
  assert.throws(() => stopDecision({ code: 0, stdout: '{"decision":"block","reason":" "}', stderr: "" }));
});
