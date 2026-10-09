const steps = ['Изучить структуру проекта', 'Найти разбор конфигурации', 'Добавить флаг strict', 'Обновить тесты', 'Проверить сборку'];

export function createServices({ signal }) {
  const listeners = new Set();
  let tick = 2, timer;
  const state = { session_dir: null, workspace: 'atlas', model: 'gpt-5.6', mode: 'По правилам', reasoning: 'medium', status: 'думает', events: 42, tools: 6, pending: 0, plan: [], context: { used: 61200, max: 272000, trigger: 244800 } };
  function advance() {
    const done = Math.min(steps.length, Math.floor(tick / 2));
    state.plan = steps.map((step, index) => ({ step, status: index < done ? 'completed' : index === done ? 'in_progress' : 'pending' }));
    state.status = done === steps.length ? 'ожидает' : ['думает', 'выполняет действие', 'пишет'][tick % 3];
    if (done < steps.length) { state.events += 3; state.tools += tick % 2; }
    for (const notify of listeners) notify();
    tick = done === steps.length && tick % 2 === 1 ? 2 : tick + 1;
  }
  advance();
  signal.addEventListener('abort', () => { clearInterval(timer); listeners.clear(); }, { once: true });
  return { 'agent.session.read': view => {
    const check = () => { signal.throwIfAborted(); view.throwIfAborted(); };
    return {
      read() { check(); return structuredClone(state); },
      subscribe(callback) {
        check(); const notify = () => callback(structuredClone(state));
        listeners.add(notify); notify(); timer ??= setInterval(advance, 1600);
        const stop = () => listeners.delete(notify);
        view.addEventListener('abort', stop, { once: true }); return stop;
      },
    };
  } };
}
