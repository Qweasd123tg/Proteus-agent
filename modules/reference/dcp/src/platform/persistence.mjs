import { AsyncLocalStorage } from "node:async_hooks";
import { readFile, mkdir, writeFile, rename, unlink } from "node:fs/promises";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { serializePruneMessagesState } from "../../node_modules/@tarquinen/opencode-dcp/lib/state/utils.ts";
import { validateEnvelope } from "./persisted-state.mjs";

const scopes = new AsyncLocalStorage();
const scope = () => {
  const value = scopes.getStore();
  if (!value) throw new Error("DCP persistence outside invocation");
  value.signal.throwIfAborted();
  return value;
};
const file = (session) => {
  if (!/^[a-zA-Z0-9_-]+$/.test(session)) throw new Error("invalid conversation session id");
  return join(scope().directory, `${session}.json`);
};

export async function transaction(directory, signal, action) {
  const context = { directory, signal, pending: new Map() };
  return scopes.run(context, async () => {
    const result = await action();
    signal.throwIfAborted();
    for (const [session, state] of context.pending) {
      const path = file(session);
      await mkdir(directory, { recursive: true });
      const temporary = `${path}.${randomUUID()}.tmp`;
      try {
        await writeFile(temporary, JSON.stringify({ version: 1, state }), { signal, mode: 0o600 });
        signal.throwIfAborted();
        await rename(temporary, path);
      } finally {
        await unlink(temporary).catch((error) => { if (error.code !== "ENOENT") throw error; });
      }
    }
    return result;
  });
}

export async function saveSessionState(state) {
  if (!state.sessionId) return;
  scope().pending.set(state.sessionId, structuredClone({
    manualMode: false,
    prune: { tools: Object.fromEntries(state.prune.tools), messages: serializePruneMessagesState(state.prune.messages) },
    nudges: Object.fromEntries(Object.entries(state.nudges).map(([key, set]) => [key, [...set]])),
    stats: state.stats,
    lastUpdated: new Date().toISOString(),
  }));
}

export async function loadSessionState(session) {
  if (scope().pending.has(session)) return structuredClone(scope().pending.get(session));
  let envelope;
  try { envelope = JSON.parse(await readFile(file(session), { encoding: "utf8", signal: scope().signal })); }
  catch (error) { if (error.code === "ENOENT") return null; throw error; }
  return validateEnvelope(envelope);
}
export async function loadManualModeSetting(session) { return (await loadSessionState(session))?.manualMode; }
export async function saveManualModeSetting() { throw new Error("OpenCode manual mode is unsupported"); }
export async function loadAllSessionStats() { throw new Error("OpenCode stats command is unsupported"); }
