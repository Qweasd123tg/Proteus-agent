import { sessionStateService } from "./session-state.js";
// Settings/composer mounts can outlive a session's WASM callback bindings.
export function createAgentServices() {
  let current;
  const waiting = new Set();
  function connected(signal) {
    return new Promise((resolve, reject) => {
      const cancel = () => {
        waiting.delete(ready);
        reject(signal.reason);
      };
      const ready = (binding) => {
        signal.removeEventListener("abort", cancel);
        waiting.delete(ready);
        resolve(binding);
      };
      waiting.add(ready);
      signal.addEventListener("abort", cancel, { once: true });
    });
  }
  async function read(name, args, signal) {
    signal.throwIfAborted();
    const binding = current ?? (await connected(signal));
    signal.throwIfAborted();
    if (binding !== current) throw Error("Сессия изменилась");
    const value = await binding[name](...args, signal);
    signal.throwIfAborted();
    if (binding !== current) throw Error("Сессия изменилась");
    return JSON.parse(value);
  }
  const reader = (name) => (signal) =>
    Object.freeze({ read: () => read(name, [], signal) });
  const services = {
    "agent.config.read": reader("readConfig"),
    "agent.model.quota.read": reader("readQuota"),
    "agent.usage.read": reader("readUsage"),
    "agent.session.read": sessionStateService,
    "agent.workspace.read": (signal) =>
      Object.freeze({
        list: (path) =>
          read(
            "readWorkspace",
            ["/workspace/list?path=" + encodeURIComponent(path)],
            signal,
          ),
        read: (path) =>
          read(
            "readWorkspace",
            ["/workspace/file?path=" + encodeURIComponent(path)],
            signal,
          ),
        changes: () => read("readWorkspace", ["/workspace/changes"], signal),
        diff: (path) =>
          read(
            "readWorkspace",
            ["/workspace/diff?path=" + encodeURIComponent(path)],
            signal,
          ),
      }),
  };
  return {
    services,
    bind(binding) {
      current = binding;
      for (const ready of waiting) ready(binding);
      return () => {
        if (current === binding) current = undefined;
      };
    },
  };
}
