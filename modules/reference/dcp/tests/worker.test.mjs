import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

test("wire v3 multiplexes current hook/tool exports and cancellation tolerates a late read-only callback", { timeout: 8000 }, async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "proteus-dcp-worker-"));
  const child = spawn(process.execPath, [new URL("../dist/worker.js", import.meta.url).pathname]);
  const closed = once(child, "close");
  let errors = "";
  child.stderr.setEncoding("utf8").on("data", (data) => { errors += data; });
  const waiters = new Map();
  const callbacks = [];
  const lines = createInterface({ input: child.stdout });
  let onCallback;
  lines.on("line", (line) => {
    const frame = JSON.parse(line);
    if (frame.method) { callbacks.push(frame); onCallback?.(frame); }
    else { waiters.get(frame.id)?.(frame); waiters.delete(frame.id); }
  });
  const write = (frame) => child.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...frame }) + "\n");
  const send = (id, method, params) => new Promise((resolve) => { waiters.set(id, resolve); write({ id, method, params }); });
  const binding = (slot, module_id) => ({ slot, module_id, contract_version: slot === "hook" ? "v4" : "v5", composition: "ordered_many", module_config: { state_dir: directory }, host_features: [] });
  const envelope = (id, slot, module_id, params) => ({ export: { slot, module_id }, lineage: { root_invocation_id: id, parent_invocation_id: null, depth: 0 }, params });
  const list = (id) => send(id, "list", envelope(id, "tool", "dcp.tools", null));
  t.after(async () => { child.stdin.end(); await closed; lines.close(); await rm(directory, { recursive: true }); });
  const initialized = await send("h:7:0", "initialize", { protocol_version: "v3", component_id: "dcp", exports: [binding("hook", "hook.dcp"), binding("tool", "dcp.tools")] });
  assert.equal(initialized.result.exports.length, 2, errors);
   const tools = (await list("h:7:1")).result.result;
   assert.equal(tools[0].spec.name, "compress");
   assert.equal(tools[0].model_visible, true);
   assert.deepEqual(tools[1].user_command.name, "dcp");
   assert.equal(tools[1].model_visible, false);
  const nextCallback = new Promise((resolve) => { onCallback = resolve; });
  const slow = send("h:7:2", "invoke", envelope("h:7:2", "tool", "dcp.tools", {
    cwd: directory, attribution: { execution_id: "e", agent: { session_id: "s", thread_id: "t", turn_id: "u" } },
    call: { id: "c", name: "compress", surface: "function", raw_arguments: null, args: { topic: "old", content: [{ startId: "@1@", endId: "@2@", summary: "done" }] } },
  }));
  const callback = await nextCallback;
  assert.equal(callback.method, "host.conversation.read");
  assert.deepEqual(callback.params, { invocation_id: "h:7:2", params: {} });
   assert.equal((await list("h:7:3")).result.result[0].spec.name, "compress");
  write({ method: "$/cancelRequest", params: { invocation_id: "h:7:2", cause: "timeout" } });
  assert.equal((await slow).error.code, -32800);
  write({ id: callback.id, result: {} });
   assert.equal((await list("h:7:4")).result.result[0].spec.name, "compress");
  assert.match((await list("h:6:5")).error.message, /generation/);
  const detached = await send("h:7:5", "hook.invoke", envelope("h:7:5", "hook", "hook.dcp", {
    cwd: directory, attribution: { execution_id: "detached", agent: null }, conversation: null,
    event: { event: "before_model", origin: "direct", request: {} },
  }));
  assert.deepEqual(detached.result.result, { action: "continue" });
  assert.equal(callbacks.length, 1, "hook must not call host services");
  assert.deepEqual(await readdir(directory), [], "canceled read must not create state");
});
