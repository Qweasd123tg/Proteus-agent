export function createServices({ signal }) {
  const state = { session_dir: null, workspace: 'atlas', model: 'gpt-5.6', mode: 'По правилам', reasoning: 'medium', status: 'думает', events: 42, tools: 6, pending: 0, plan: [], context: { used: 61200, max: 272000, trigger: 244800 } };
  const listeners = new Set();
  const timer = setInterval(() => {
    state.context.used = state.context.used + 5400 > state.context.trigger ? 61200 : state.context.used + 5400;
    for (const notify of listeners) notify();
  }, 1600);
  signal.addEventListener('abort', () => { clearInterval(timer); listeners.clear(); }, { once: true });
  return { 'agent.session.read': view => ({
    read() { signal.throwIfAborted(); view.throwIfAborted(); return structuredClone(state); },
    subscribe(callback) {
      signal.throwIfAborted(); view.throwIfAborted();
      const notify = () => callback(structuredClone(state)); listeners.add(notify); notify();
      const stop = () => listeners.delete(notify); view.addEventListener('abort', stop, { once: true }); return stop;
    },
  }) };
}
