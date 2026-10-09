import { randomUUID } from "node:crypto";

const zero = () => ({ input: 0, output: 0, reasoning: 0, cache: { read: 0, write: 0 } });
export const textPart = (text) => ({ part_id: randomUUID(), provenance: "runtime", scope: "request", payload: { Text: { text } } });
const changed = (part, payload) => JSON.stringify(part.payload) === JSON.stringify(payload)
  ? part : { ...part, part_id: randomUUID(), scope: "request", payload };

// Bookkeeping projection only. Unknown/provider parts remain canonical and immutable.
export function project(native, durable, session, model = {}, observations = []) {
  const ids = new Set(durable.map((message) => message.id));
  const results = new Map();
  const owners = new Map();
  const links = new Map();
  const originals = new Map();
  const messages = [];
  const info = (message, index) => ({
    id: message.id, role: message.role.toLowerCase(), sessionID: session,
    time: { created: index + 1 }, agent: "proteus", model: { providerID: model.provider, modelID: model.model },
    providerID: model.provider, modelID: model.model, tokens: zero(),
    summary: message.parts.length > 0 && message.parts.every((part) => part.provenance === "compactor"),
  });
  for (const message of native) for (const part of message.parts) {
    if (part.payload.ToolResult) results.set(part.payload.ToolResult.result.call_id, part.payload.ToolResult.result);
    if (part.payload.ToolCall) owners.set(part.payload.ToolCall.call.id, message.id);
  }
  for (const [index, message] of native.entries()) {
    if (!ids.has(message.id) || !["User", "Assistant"].includes(message.role)) continue;
    const base = { sessionID: session, messageID: message.id };
    const parts = [];
    const metadata = info(message, index);
    if (metadata.summary) metadata.role = "assistant";
    else if (message.role === "Assistant") parts.push({ ...base, id: `${message.id}:step`, type: "step-start" });
    for (const part of message.parts) {
      let projected;
      if (part.payload.Text) projected = { ...base, id: part.part_id, type: "text", text: part.payload.Text.text };
      if (part.payload.Image) projected = { ...base, id: part.part_id, type: "file", mime: part.payload.Image.image.mime_type, url: "" };
      if (part.payload.ToolCall) {
        const call = part.payload.ToolCall.call;
        const result = results.get(call.id);
        projected = { ...base, id: call.id, type: "tool", callID: call.id, tool: call.name,
          state: { status: !result ? "running" : result.ok ? "completed" : "error", input: structuredClone(call.args),
            output: result?.ok ? result.output : undefined, error: result?.error, metadata: result?.metadata ?? {} } };
      }
      if (projected) { parts.push(projected); links.set(part, projected); }
    }
    const projected = { info: metadata, parts };
    messages.push(projected);
    originals.set(message.id, projected);
  }
  for (const message of native) for (const part of message.parts) {
    const result = part.payload.ToolResult?.result;
    if (result) {
      const owner = originals.get(owners.get(result.call_id));
      const link = owner?.parts.find((part) => part.type === "tool" && part.callID === result.call_id);
      if (link) links.set(part, link);
    }
  }
  const facts = observations.slice(observations.findLastIndex((fact) => fact.kind === "history_compacted") + 1);
  const lastUsage = facts.findLast((fact) => fact.kind === "usage");
  const lastAssistant = messages.findLast((message) => message.info.role === "assistant" && !message.info.summary);
  // Only aggregate canonical usage is available; upstream consumes its sum.
  if (lastUsage && lastAssistant) lastAssistant.info.tokens.output = lastUsage.last_tokens;

  function restore() {
    const retained = new Set(messages.map((message) => message.info.id));
    const linked = new Set(links.values());
    const output = [];
    let next = 0;
    const synthetic = () => {
      while (next < messages.length && !originals.has(messages[next].info.id)) {
        const message = messages[next++];
        output.push({ id: randomUUID(), role: "User", phase: null, name: null, tool_call_id: null, metadata: null,
          parts: message.parts.filter((part) => part.type === "text").map((part) => textPart(part.text)) });
      }
    };
    for (const message of native) {
      const projected = originals.get(message.id);
      if (projected) { synthetic(); if (!retained.has(message.id)) continue; next++; }
      const parts = [];
      const added = projected?.parts.filter((part) => part.type === "text" && !linked.has(part)).map((part) => textPart(part.text)) ?? [];
      for (const part of message.parts) {
        if (part.payload.ToolCall) parts.push(...added.splice(0));
        const result = part.payload.ToolResult?.result;
        const owner = result && owners.get(result.call_id);
        if (result && originals.has(owner) && !retained.has(owner)) continue;
        const edit = links.get(part);
        if (part.payload.Text && edit?.type === "text") parts.push(changed(part, { Text: { text: edit.text } }));
        else if (part.payload.ToolCall && edit?.type === "tool") {
          const call = part.payload.ToolCall.call;
          const altered = JSON.stringify(call.args) !== JSON.stringify(edit.state.input);
          parts.push(changed(part, { ToolCall: { call: { ...call, args: edit.state.input, raw_arguments: altered ? null : call.raw_arguments } } }));
        } else if (result && edit?.type === "tool" && edit.state.status === "completed" &&
          typeof edit.state.output === "string" && edit.state.output !== result.output) {
          parts.push(changed(part, { ToolResult: { result: { ...result, output: edit.state.output, content: [] } } }));
        } else parts.push(part);
      }
      parts.push(...added);
      if (parts.length || !message.parts.length) output.push({ ...message, parts });
    }
    synthetic();
    return output;
  }
  return { messages, restore, summaryBase: { info: info({ id: "msg_dcp_base", role: "User", parts: [] }, 0), parts: [] } };
}
