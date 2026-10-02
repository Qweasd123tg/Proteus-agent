#!/usr/bin/env node
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { Console } from "node:console";
import { createHooks } from "./hooks.mjs";
import { hostId, nonblank, object } from "./validation.mjs";

// A transplanted console.log must not corrupt the component's JSON-lines stdout.
globalThis.console = new Console({ stdout: process.stderr, stderr: process.stderr });
const send = (frame) => process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...frame })}\n`);
const error = (id, code, message) => send({ id, error: { code, message } });
const active = new Map();
const exports = new Map();
let generation;
let ready = false;

async function initialize(request) {
  object(request.params, ["protocol_version", "component_id", "exports"], [], "initialize");
  const params = request.params;
  if (params.protocol_version !== "v3") throw new Error("only component protocol v3 is supported");
  nonblank(params.component_id, "component id");
  if (!Array.isArray(params.exports) || !params.exports.length) throw new Error("at least one hook export is required");
  generation = /^h:(0|[1-9]\d*):0$/.exec(request.id)?.[1];
  if (generation === undefined) throw new Error("initialize requires h:<generation>:0 id");
  const manifest = [];
  for (const binding of params.exports) {
    object(binding, ["slot", "module_id", "contract_version", "composition", "module_config", "host_features"], [], "export binding");
    if (binding.slot !== "hook" || binding.contract_version !== "v3" || binding.composition !== "ordered_many") {
      throw new Error("worker supports only hook/v3 ordered_many exports");
    }
    nonblank(binding.module_id, "module id");
    if (exports.has(binding.module_id)) throw new Error("duplicate export module id");
    if (!Array.isArray(binding.host_features) || binding.host_features.length) throw new Error("hook export has no host features");
    const config = object(binding.module_config, ["entry"], ["settings"], "hook module config");
    nonblank(config.entry, "hook entry");
    const hooks = createHooks(config.settings === undefined ? {} : config.settings);
    const extension = await import(pathToFileURL(resolve(config.entry)).href);
    if (typeof extension.default !== "function") throw new Error("hook entry must export a default setup function");
    await extension.default(hooks.api);
    hooks.seal();
    exports.set(binding.module_id, hooks);
    manifest.push({ slot: "hook", module_id: binding.module_id, contract_version: "v3", composition: "ordered_many", module_features: [] });
  }
  ready = true;
  send({ id: request.id, result: { protocol_version: "v3", component_id: params.component_id, exports: manifest } });
}

function cancel(request) {
  object(request, ["jsonrpc", "method", "params"], [], "cancel notification");
  object(request.params, ["invocation_id", "cause"], [], "cancel params");
  hostId(request.params.invocation_id, generation);
  if (!["user", "timeout", "shutdown"].includes(request.params.cause)) throw new Error("invalid cancel cause");
  active.get(request.params.invocation_id)?.controller.abort(new Error(`hook canceled: ${request.params.cause}`));
}

function invoke(request) {
  hostId(request.id, generation);
  if (request.method !== "hook.invoke") throw new Error("unsupported hook method");
  if (active.has(request.id)) throw new Error("duplicate active invocation id");
  const params = object(request.params, ["export", "lineage", "params"], [], "invocation");
  object(params.export, ["slot", "module_id"], [], "export ref");
  const hooks = params.export.slot === "hook" && exports.get(params.export.module_id);
  if (!hooks) throw new Error("invocation targets an unconfigured export");
  const lineage = object(params.lineage, ["root_invocation_id", "parent_invocation_id", "depth"], [], "lineage");
  hostId(lineage.root_invocation_id, generation);
  if (!Number.isSafeInteger(lineage.depth) || lineage.depth < 0) throw new Error("invalid lineage depth");
  if (lineage.depth === 0) {
    if (lineage.root_invocation_id !== request.id || lineage.parent_invocation_id !== null) throw new Error("invalid root lineage");
  } else {
    hostId(lineage.parent_invocation_id, generation);
    const parent = active.get(lineage.parent_invocation_id);
    if (!parent || lineage.root_invocation_id !== parent.lineage.root_invocation_id || lineage.depth !== parent.lineage.depth + 1) {
      throw new Error("lineage parent is not active in this component");
    }
  }
  const controller = new AbortController();
  active.set(request.id, { controller, lineage });
  // Keep reading stdin while each invocation awaits its own handlers.
  Promise.resolve().then(() => hooks.invoke(params.params, controller.signal)).then(
    (result) => send({ id: request.id, result }),
    (failure) => error(request.id, controller.signal.aborted ? -32800 : -32000, String(failure.message ?? failure)),
  ).finally(() => active.delete(request.id));
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  let request;
  try {
    request = JSON.parse(line);
    if (request.jsonrpc !== "2.0") throw new Error("jsonrpc must be 2.0");
    if (ready && request.method === "$/cancelRequest") {
      cancel(request);
      continue;
    }
    object(request, ["jsonrpc", "id", "method", "params"], [], "request");
    if (!ready) {
      if (request.method !== "initialize") throw new Error("first request must be initialize");
      await initialize(request);
    } else invoke(request);
  } catch (failure) {
    if (typeof request?.id === "string") error(request.id, -32602, String(failure.message ?? failure));
    else console.error(failure);
    if (!ready || typeof request?.id !== "string") {
      process.exitCode = 1;
      break;
    }
  }
}
for (const { controller } of active.values()) controller.abort(new Error("component stdin closed"));
