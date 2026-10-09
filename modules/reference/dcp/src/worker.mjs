#!/usr/bin/env node
import { createInterface } from "node:readline";
import { Console } from "node:console";
import { createDcp } from "./module.mjs";
import { configuration } from "./config.mjs";

globalThis.console = new Console({ stdout: process.stderr, stderr: process.stderr });
const send = (frame) => process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...frame })}\n`);
const object = (value, keys, label) => {
  if (!value || typeof value !== "object" || Array.isArray(value) ||
    Object.keys(value).some((key) => !keys.includes(key)) || keys.some((key) => !(key in value))) throw new Error(`invalid ${label}`);
  return value;
};
const active = new Map();
const pending = new Map();
const canceledCallbacks = new Set();
const used = new Set();
const exports = new Map();
let generation;
let sequence = 0;
let ready = false;
const wireId = (id, direction, zero = false) => {
  const match = /^(h|m):(0|[1-9]\d*):(0|[1-9]\d*)$/.exec(id);
  if (!match || match[1] !== direction || match[2] !== generation || (!zero && match[3] === "0") ||
    BigInt(match[2]) > 18446744073709551615n || BigInt(match[3]) > 18446744073709551615n) throw new Error("invalid wire id/generation");
};
const configSchema = { fields: [
  ["state_dir", "State directory", { type: "string", multiline: false, secret: false }],
  ["debug", "Debug", { type: "boolean" }],
  ...["compress", "strategies", "turnProtection", "protectedFilePatterns"].map((key) => [key, key, { type: "json" }]),
].map(([key, title, value]) => ({ key, title, description: "Настройки реализации DCP; одинаковые у hook и tool exports.",
  value, default: null, required: false, advanced: key !== "compress", unit: null })) };

async function initialize(frame) {
  object(frame.params, ["protocol_version", "component_id", "exports"], "initialize");
  const input = frame.params;
  generation = /^h:(0|[1-9]\d*):0$/.exec(frame.id)?.[1];
  if (generation === undefined || input.protocol_version !== "v3" || typeof input.component_id !== "string" || !input.component_id.trim()) throw new Error("invalid initialization");
  wireId(frame.id, "h", true);
  if (!Array.isArray(input.exports) || !input.exports.length) throw new Error("no exports configured");
  let canonical;
  let module;
  const manifest = [];
  for (const binding of input.exports) {
    object(binding, ["slot", "module_id", "contract_version", "composition", "module_config", "host_features"], "binding");
    const expected = binding.slot === "hook" ? "hook.dcp" : binding.slot === "tool" ? "dcp.tools" : null;
    if (!expected || binding.module_id !== expected || binding.contract_version !== "v4" || binding.composition !== "ordered_many" ||
      !Array.isArray(binding.host_features) || binding.host_features.length || exports.has(binding.slot)) throw new Error("unsupported DCP export");
    const settings = configuration(binding.module_config);
    const normalized = JSON.stringify(settings);
    if (canonical !== undefined && canonical !== normalized) throw new Error("DCP hook/tool settings must be identical");
    canonical = normalized;
    module ??= createDcp(binding.module_config);
    exports.set(binding.slot, { id: expected, module });
    manifest.push({ slot: binding.slot, module_id: expected, contract_version: "v4", composition: "ordered_many", module_features: [], config_schema: configSchema });
  }
  ready = true;
  send({ id: frame.id, result: { protocol_version: "v3", component_id: input.component_id, exports: manifest } });
}

function callback(invocation, signal) {
  signal.throwIfAborted();
  const id = `m:${generation}:${++sequence}`;
  return new Promise((resolve, reject) => {
    const canceled = () => { pending.delete(id); canceledCallbacks.add(id); reject(signal.reason); };
    signal.addEventListener("abort", canceled, { once: true });
    pending.set(id, { resolve, reject, cleanup: () => signal.removeEventListener("abort", canceled) });
    send({ id, method: "host.conversation.read", params: { invocation_id: invocation, params: {} } });
  });
}

function invoke(frame) {
  wireId(frame.id, "h");
  if (used.has(frame.id)) throw new Error("reused invocation id");
  used.add(frame.id);
  const envelope = object(frame.params, ["export", "lineage", "params"], "invocation");
  object(envelope.export, ["slot", "module_id"], "export reference");
  const target = exports.get(envelope.export.slot);
  if (!target || target.id !== envelope.export.module_id) throw new Error("unconfigured export");
  const lineage = object(envelope.lineage, ["root_invocation_id", "parent_invocation_id", "depth"], "lineage");
  wireId(lineage.root_invocation_id, "h");
  if (!Number.isSafeInteger(lineage.depth) || lineage.depth < 0) throw new Error("invalid lineage depth");
  if (lineage.depth === 0) {
    if (lineage.root_invocation_id !== frame.id || lineage.parent_invocation_id !== null) throw new Error("invalid root lineage");
  } else {
    wireId(lineage.parent_invocation_id, "h");
    const parent = active.get(lineage.parent_invocation_id);
    if (!parent || parent.lineage.root_invocation_id !== lineage.root_invocation_id || parent.lineage.depth + 1 !== lineage.depth) throw new Error("inactive lineage parent");
  }
  const hook = envelope.export.slot === "hook";
  if (hook ? frame.method !== "hook.invoke" : !["list", "invoke"].includes(frame.method)) throw new Error("unsupported export method");
  if (hook) object(envelope.params, ["event", "attribution", "cwd", "conversation"], "hook input");
  else if (frame.method === "invoke") object(envelope.params, ["call", "cwd", "attribution"], "tool input");
  else if (envelope.params !== null) throw new Error("list input must be null");
  const controller = new AbortController();
  active.set(frame.id, { controller, lineage });
  Promise.resolve().then(async () => ({ result: hook
    ? await target.module.hook(envelope.params, controller.signal)
    : frame.method === "list" ? [target.module.spec]
      : await target.module.invoke(envelope.params, () => callback(frame.id, controller.signal), controller.signal) })).then(
    (result) => send({ id: frame.id, result }),
    (error) => send({ id: frame.id, error: { code: controller.signal.aborted ? -32800 : -32000, message: String(error.message ?? error) } }),
  ).finally(() => active.delete(frame.id));
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  let frame;
  try {
    frame = JSON.parse(line);
    if (frame.jsonrpc !== "2.0") throw new Error("invalid JSON-RPC version");
    if ("result" in frame || "error" in frame) {
      object(frame, ["jsonrpc", "id", "result" in frame ? "result" : "error"], "callback response");
      wireId(frame.id, "m");
      if (canceledCallbacks.delete(frame.id)) continue; // A host may settle an already-canceled callback.
      const waiter = pending.get(frame.id);
      if (!waiter) throw new Error("unmatched callback response");
      pending.delete(frame.id); waiter.cleanup();
      if (frame.error) waiter.reject(new Error(frame.error.message)); else waiter.resolve(frame.result);
    } else if (ready && frame.method === "$/cancelRequest") {
      object(frame, ["jsonrpc", "method", "params"], "cancel notification");
      object(frame.params, ["invocation_id", "cause"], "cancel parameters");
      wireId(frame.params.invocation_id, "h");
      if (!["user", "timeout", "shutdown"].includes(frame.params.cause)) throw new Error("invalid cancellation cause");
      active.get(frame.params.invocation_id)?.controller.abort(new Error(`invocation canceled: ${frame.params.cause}`));
    } else {
      object(frame, ["jsonrpc", "id", "method", "params"], "request");
      if (!ready) { if (frame.method !== "initialize") throw new Error("initialize first"); await initialize(frame); }
      else invoke(frame);
    }
  } catch (error) {
    if (typeof frame?.id === "string" && frame.id.startsWith("h:")) send({ id: frame.id, error: { code: -32602, message: String(error.message ?? error) } });
    else { console.error(error); process.exitCode = 1; break; }
    if (!ready) { process.exitCode = 1; break; }
  }
}
for (const { controller } of active.values()) controller.abort(new Error("component stdin closed"));
