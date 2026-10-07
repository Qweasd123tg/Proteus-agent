import { sessionStateService } from "./session-state.js";
// Settings/composer mounts can outlive a session's WASM callback bindings.
export function createAgentServices() {
  let current;
  const changes = new Set();
  const subscribe = (signal, callback) => {
    signal.throwIfAborted();
    changes.add(callback);
    const stop = () => { changes.delete(callback); signal.removeEventListener("abort",stop); };
    signal.addEventListener("abort", stop, {once:true});
    // A retained settings store may predate this session binding.
    callback(null);
    return stop;
  };
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
    // Saving validates and builds the complete assembly on the server before
    // the profile file and the running agent change.
    "agent.config.builder": (signal) =>
      Object.freeze({
        read: () => read("readConfigBuilder", [], signal),
        save: (request) =>
          read("saveConfigBuilder", [JSON.stringify(request)], signal),
        history: () => read("readConfigHistory", [], signal),
        subscribe: (callback) => subscribe(signal, callback),
      }),
    "agent.model.quota.read": reader("readQuota"),
    "agent.addons": (signal) => Object.freeze({
      read: () => read("readAddons", [], signal),
      save: (request) => read("saveAddons", [JSON.stringify(request)], signal),
      subscribe: (callback) => subscribe(signal, callback),
    }),
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
    configurationChanged(error) {
      for (const callback of [...changes]) callback(error);
    },
    bind(binding) {
      current = binding;
      for (const ready of waiting) ready(binding);
      for (const callback of [...changes]) callback(null);
      return () => {
        if (current === binding) current = undefined;
      };
    },
  };
}
