import test from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createDcp } from "../dist/module.js";

const part = (payload) => ({ part_id: randomUUID(), provenance: "model", scope: "conversation", payload });
const message = (role, parts) => ({ id: randomUUID(), role, phase: null, parts, name: null, tool_call_id: null, metadata: {} });
const text = (role, value) => message(role, [part({ Text: { text: value } })]);
const contents = (messages) => messages.flatMap((msg) => msg.parts.map((p) => p.payload.Text?.text ?? p.payload.ToolResult?.result.output ?? "")).join("\n");
const signal = () => new AbortController().signal;
const ref = (id) => String.fromCharCode(64) + id + String.fromCharCode(64);
function fixture() {
  const call = { id: "compress-call", name: "compress", args: {}, surface: "function", raw_arguments: null };
  const messages = [text("User", "Keep <protect>CRITICAL EXACT VALUE</protect>"), text("Assistant", "long obsolete investigation ".repeat(80)),
    text("User", "now finish"), message("Assistant", [part({ ToolCall: { call } })])];
  return { call, messages, conversation: { messages, model_context: [] }, attribution: { execution_id: randomUUID(), agent: { session_id: randomUUID(), thread_id: randomUUID(), turn_id: randomUUID() } } };
}
const input = (f, spec) => ({ attribution: f.attribution, cwd: process.cwd(), conversation: f.conversation,
  event: { event: "before_model", origin: "direct", request: { model: { provider: "fake", model: "fake" }, messages: f.messages,
    tools: [spec], instructions: [{ kind: "System", text: "owner instruction", priority: 100 }], limits: { max_input_tokens: 128000, max_output_tokens: null } } } });
const toolInput = (f, args) => ({ cwd: process.cwd(), attribution: f.attribution, skills: { disabled: [], packages: [] }, call: { ...f.call, args } });
async function temporary(t) {
  const directory = await mkdtemp(join(tmpdir(), "proteus-dcp-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

test("upstream range compression preserves protected content and survives a new process state", async (t) => {
  const directory = await temporary(t);
  const module = createDcp({ state_dir: directory, compress: { protectTags: true } });
  const f = fixture();
  const original = structuredClone(f.messages);
  const first = await module.hook(input(f, module.spec), signal());
  assert.equal(first.action, "model_context");
  assert.match(contents(first.messages), /@2@/);
  assert.match(first.instructions.at(-1).text, /compress/);
  const result = await module.invoke(toolInput(f, { topic: "finished research", content: [{ startId: "@1@", endId: "@2@", summary: "Research complete." }] }),
    async () => ({ conversation: f.conversation, message_id: f.messages.at(-1).id }), signal());
  assert.equal(result.ok, true);
  const fresh = createDcp({ state_dir: directory, compress: { protectTags: true } });
  const final = await fresh.hook(input(f, fresh.spec), signal());
  assert.match(contents(final.messages), /Research complete/);
  assert.match(contents(final.messages), /CRITICAL EXACT VALUE/);
  assert.doesNotMatch(contents(final.messages), /long obsolete investigation/);
  assert.deepEqual(f.messages, original);
  assert.deepEqual(final.messages.at(-1).parts.find((p) => p.payload.ToolCall), original.at(-1).parts[0]);
  assert.equal(JSON.parse(await readFile(join(directory, `${f.attribution.agent.session_id}.json`))).version, 1);
});

test("message mode uses original upstream selection", async (t) => {
  const module = createDcp({ state_dir: await temporary(t), compress: { mode: "message" } });
  const f = fixture();
  await module.hook(input(f, module.spec), signal());
  const result = await module.invoke(toolInput(f, { topic: "finished", content: [{ messageId: "@2@", topic: "research", summary: "Research complete." }] }),
    async () => ({ conversation: f.conversation, message_id: f.messages.at(-1).id }), signal());
  assert.equal(result.ok, true);
  assert.doesNotMatch(contents((await module.hook(input(f, module.spec), signal())).messages), /long obsolete investigation/);
});

test("user-only DCP commands report and restore persisted blocks without a model turn", async (t) => {
  const directory = await temporary(t);
  const module = createDcp({ state_dir: directory });
  const f = fixture();
  await module.hook(input(f, module.spec), signal());
  await module.invoke(toolInput(f, { topic: "old", content: [{ startId: ref("1"), endId: ref("2"), summary: "Research complete." }] }),
    async () => ({ conversation: f.conversation, message_id: f.messages.at(-1).id }), signal());
  const original = structuredClone(f.messages);
  const management = async (args, instance = module) => instance.invoke({ cwd: process.cwd(),
    attribution: { execution_id: randomUUID(), agent: null },
    call: { ...f.call, name: "dcp", args: { arguments: args } } }, async (method) => {
      assert.equal(method, "host.conversation.snapshot");
      return { session_id: f.attribution.agent.session_id, conversation: f.conversation };
    }, signal());
  assert.deepEqual(module.tools.map((t) => [t.spec.name, t.model_visible, t.user_command?.name]),
    [["compress", true, undefined], ["dcp", false, "dcp"]]);
  const file = join(directory, `${f.attribution.agent.session_id}.json`);
  const before = await readFile(file, "utf8");
  assert.match((await management("stats")).output, /DCP Statistics/);
  assert.match((await management("context")).output, /Current context/);
  assert.match((await management("decompress")).output, /1/);
  assert.match((await management("decompress 999")).output, /does not exist/);
  assert.equal(await readFile(file, "utf8"), before, "read-only commands must not publish state");
  await assert.rejects(management("unknown"), /unsupported/);
  await assert.rejects(management("stats extra"), /arguments/);
  const restored = await management("decompress 1");
  assert.match(restored.output, /restored|Restored/);
  const fresh = createDcp({ state_dir: directory });
  assert.match(contents((await fresh.hook(input(f, fresh.spec), signal())).messages), /long obsolete investigation/);
  assert.match((await management("decompress 1", fresh)).output, /not active/);
  assert.deepEqual(f.messages, original);
});

test("invalid ranges and cancellation do not publish compression state", async (t) => {
  const directory = await temporary(t);
  const module = createDcp({ state_dir: directory });
  const f = fixture();
  await module.hook(input(f, module.spec), signal());
  const file = join(directory, `${f.attribution.agent.session_id}.json`);
  const prior = await readFile(file, "utf8");
  const read = async () => ({ conversation: f.conversation, message_id: f.messages.at(-1).id });
  await assert.rejects(module.invoke(toolInput(f, { topic: "bad", content: [{ startId: "@2@", endId: "@1@", summary: "bad" }] }), read, signal()));
  assert.equal(await readFile(file, "utf8"), prior);
  const controller = new AbortController();
  await assert.rejects(module.invoke(toolInput(f, { topic: "canceled", content: [{ startId: "@1@", endId: "@2@", summary: "gone" }] }),
    async () => { controller.abort(new Error("canceled")); return read(); }, controller.signal), /canceled/);
  assert.equal(await readFile(file, "utf8"), prior);
  assert.deepEqual(await readdir(directory), [`${f.attribution.agent.session_id}.json`]);
});

test("nested upstream blocks survive persistence and restore without orphan tool results", async (t) => {
  const directory = await temporary(t);
  const settings = { state_dir: directory, compress: { protectTags: true } };
  const module = createDcp(settings);
  const f = fixture();
  const oldCall = { ...f.call, id: "old-read", name: "read_file", args: { path: "notes" } };
  f.messages[1].parts.push(part({ ToolCall: { call: oldCall } }));
  f.messages.splice(2, 0, message("Tool", [part({ ToolResult: { result: { call_id: oldCall.id, ok: true,
    output: "obsolete read output", content: [], error: null, metadata: null } } })]));
  const original = structuredClone(f.messages);
  await module.hook(input(f, module.spec), signal());
  await module.invoke(toolInput(f, { topic: "first", content: [{ startId: ref("1"), endId: ref("2"), summary: "First research complete." }] }),
    async () => ({ conversation: f.conversation, message_id: f.messages.at(-1).id }), signal());
  const secondCall = { ...f.call, id: "compress-again" };
  f.messages.push(message("Assistant", [part({ ToolCall: { call: secondCall } })]));
  const fresh = createDcp(settings);
  await fresh.hook(input(f, fresh.spec), signal());
  const args = { topic: "nested", content: [{ startId: ref("b1"), endId: ref("3"),
    summary: `Earlier research: ${ref("b1")}. New decision included.` }] };
  await fresh.invoke({ ...toolInput(f, args), call: { ...secondCall, args } },
    async () => ({ conversation: f.conversation, message_id: f.messages.at(-1).id }), signal());
  const final = await createDcp(settings).hook(input(f, fresh.spec), signal());
  assert.match(contents(final.messages), /First research complete/);
  assert.match(contents(final.messages), /New decision included/);
  assert.match(contents(final.messages), /CRITICAL EXACT VALUE/);
  assert.doesNotMatch(contents(final.messages), /obsolete investigation|obsolete read output/);
  assert.ok(!final.messages.some((msg) => msg.parts.some((p) => p.payload.ToolResult?.result.call_id === oldCall.id)));
  assert.deepEqual(f.messages.slice(0, -1), original);
});

test("detached invocations and unknown config/storage forms fail explicitly", async (t) => {
  assert.throws(() => createDcp({ commands: { enabled: true } }), /unsupported/);
  assert.throws(() => createDcp({ compress: { permission: "allow" } }), /policy/);
  const directory = await temporary(t);
  const module = createDcp({ state_dir: directory });
  const f = fixture();
  await assert.rejects(module.invoke({ ...toolInput(f, {}), attribution: { ...f.attribution, agent: null } }, () => { throw new Error("not read"); }, signal()), /conversation/);
  await writeFile(join(directory, `${f.attribution.agent.session_id}.json`), '{"version":0,"state":{}}');
  await assert.rejects(module.hook(input(f, module.spec), signal()), /persisted state/);
});
