// Client projection shared with every extension through an explicit read-only service.
let snapshot = {};
const listeners = new Set();
export function publishSessionState(value) {
  const next = JSON.parse(value);
  if (JSON.stringify(snapshot) === value) return;
  snapshot = next;
  for (const notify of listeners) {
    try { notify(snapshot); } catch (error) { console.error('Extension session subscriber failed', error); }
  }
}
export function sessionStateService(signal) {
  return Object.freeze({
    read: () => { signal.throwIfAborted(); return structuredClone(snapshot); },
    subscribe(callback) {
      signal.throwIfAborted();
      const notify = value => callback(structuredClone(value));
      listeners.add(notify); notify(snapshot);
      const stop = () => listeners.delete(notify);
      signal.addEventListener('abort', stop, { once: true });
      return stop;
    },
  });
}
