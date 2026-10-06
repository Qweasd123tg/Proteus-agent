// Demo implementations behind the same interfaces as the real services, for
// trying a disabled extension: fictional data, no agent requests, no writes.
// Each entry is a factory taking the view's signal, like the real map.
const now = () => Math.floor(Date.now() / 1000);
const PLAN = ['Изучить структуру проекта', 'Найти разбор конфигурации', 'Добавить флаг strict', 'Обновить тесты', 'Проверить сборку'];
const FILES = {
  'Cargo.toml': '[package]\nname = "atlas"\nversion = "0.4.0"\nedition = "2024"\n\n[dependencies]\nserde = { version = "1", features = ["derive"] }\ntoml = "0.8"\n',
  'README.md': '# Atlas\n\nНебольшая CLI-утилита для проверки конфигураций.\n\n```sh\natlas check config.toml --strict\n```\n',
  'docs/guide.md': '# Руководство\n\nФлаг `--strict` превращает предупреждения в ошибки.\n',
  'src/main.rs': 'mod config;\nmod parser;\n\nfn main() {\n    let cfg = config::load("config.toml").expect("config");\n    println!("strict = {}", cfg.strict);\n}\n',
  'src/config.rs': 'use serde::Deserialize;\n\n#[derive(Deserialize)]\npub struct Config {\n    pub name: String,\n    #[serde(default)]\n    pub strict: bool,\n}\n\npub fn load(path: &str) -> anyhow::Result<Config> {\n    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)\n}\n',
  'src/parser/mod.rs': 'pub mod tokens;\n\npub use tokens::Token;\n',
  'src/parser/tokens.rs': '#[derive(Debug, PartialEq)]\npub enum Token {\n    Key(String),\n    Value(String),\n}\n',
  'tests/config.rs': '#[test]\nfn strict_defaults_to_false() {\n    let cfg: atlas::Config = toml::from_str("name = \\"x\\"").unwrap();\n    assert!(!cfg.strict);\n}\n',
};
const CHANGES = { 'src/config.rs': 'modified', 'README.md': 'modified', 'tests/config.rs': 'added' };
const PATCHES = {
  'src/config.rs': 'diff --git a/src/config.rs b/src/config.rs\n--- a/src/config.rs\n+++ b/src/config.rs\n@@ -3,5 +3,7 @@ use serde::Deserialize;\n #[derive(Deserialize)]\n pub struct Config {\n     pub name: String,\n+    #[serde(default)]\n+    pub strict: bool,\n }\n',
  'README.md': 'diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -3,3 +3,7 @@\n Небольшая CLI-утилита для проверки конфигураций.\n+\n+```sh\n+atlas check config.toml --strict\n+```\n',
  'tests/config.rs': 'diff --git a/tests/config.rs b/tests/config.rs\nnew file mode 100644\n--- /dev/null\n+++ b/tests/config.rs\n@@ -0,0 +1,5 @@\n+#[test]\n+fn strict_defaults_to_false() {\n+    let cfg: atlas::Config = toml::from_str("name = \\"x\\"").unwrap();\n+    assert!(!cfg.strict);\n+}\n',
};

function listing(path) {
  const prefix = path ? `${path}/` : '', entries = new Map();
  for (const file of Object.keys(FILES)) {
    if (!file.startsWith(prefix)) continue;
    const [name, ...rest] = file.slice(prefix.length).split('/');
    entries.set(name, { name, path: prefix + name, kind: rest.length ? 'directory' : 'file' });
  }
  if (path && !entries.size) throw new Error('Папка не найдена');
  const sorted = [...entries.values()].sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === 'directory' ? -1 : 1));
  return { path, entries: sorted, truncated: false };
}

// The agent works through the plan while the demo is open.
function sessionScenario(signal) {
  const listeners = new Set();
  let tick = 2, timer;
  const state = { session_dir: null, workspace: 'atlas', model: 'gpt-5.6', mode: 'По правилам', reasoning: 'medium', status: 'думает', events: 42, tools: 6, pending: 0, plan: [], context: { used: 61200, max: 272000, trigger: 244800 } };
  function advance() {
    const done = Math.min(PLAN.length, Math.floor(tick / 2));
    state.plan = PLAN.map((step, index) => ({ step, status: index < done ? 'completed' : index === done ? 'in_progress' : 'pending' }));
    state.status = done === PLAN.length ? 'ожидает' : ['думает', 'выполняет действие', 'пишет'][tick % 3];
    if (done < PLAN.length) { state.events += 3; state.tools += tick % 2; state.context.used = Math.min(state.context.trigger, state.context.used + 5400); }
    for (const notify of listeners) notify();
    if (done === PLAN.length && tick % 2 === 1) { tick = 2; state.context.used = 61200; }
    else tick += 1;
  }
  advance();
  signal.addEventListener('abort', () => { clearInterval(timer); listeners.clear(); }, { once: true });
  return view => Object.freeze({
    read() { view.throwIfAborted(); return structuredClone(state); },
    subscribe(callback) {
      view.throwIfAborted();
      const notify = () => callback(structuredClone(state));
      listeners.add(notify); notify();
      timer ??= setInterval(advance, 1600);
      const stop = () => listeners.delete(notify);
      view.addEventListener('abort', stop, { once: true });
      return stop;
    },
  });
}

function stateService(initial, writable) {
  const state = structuredClone(initial), listeners = new Set();
  return view => Object.freeze({
    read() { view.throwIfAborted(); return structuredClone(state); },
    set(key, value) {
      view.throwIfAborted();
      if (!writable.includes(key)) throw new Error(`Поле нельзя изменить: ${key}`);
      state[key] = value;
      if (key === 'effort') state.effortLabel = value[0].toUpperCase() + value.slice(1);
      for (const notify of listeners) notify();
    },
    subscribe(callback) {
      view.throwIfAborted();
      listeners.add(callback);
      const stop = () => listeners.delete(callback);
      view.addEventListener('abort', stop, { once: true });
      return stop;
    },
  });
}

const reply = (view, value) => { view.throwIfAborted(); return Promise.resolve(structuredClone(value)); };
const request = (id, turn, start, input, cached, output, tools, origin = 'direct', model = 'gpt-5.6') => ({
  exchange_id: id, turn_id: turn, model: { provider: 'openai', model }, origin,
  started_at_ms: start, finished_at_ms: start + 4000 + output * 3, status: 'completed', finish_reason: 'stop',
  usage: { input_tokens: input, output_tokens: output, cached_input_tokens: cached, cache_creation_input_tokens: null, reasoning_output_tokens: Math.round(output / 3) },
  message_count: 2 + tools, tool_count: tools, reasoning_effort: 'medium', max_output_tokens: null,
});

export const demoServiceNames = new Set(['agent.session.read', 'agent.workspace.read', 'agent.model.quota.read', 'agent.usage.read', 'agent.config.read', 'client.preferences', 'client.composer']);

export function createDemoServices(signal) {
  const start = Date.now() - 18 * 60_000;
  const usage = { session_id: 'demo', revision: 7, latest_turn_id: 'turn-2', requests: [
    request('ex-1', 'turn-1', start, 18400, 0, 920, 3), request('ex-2', 'turn-1', start + 40_000, 24100, 17800, 1480, 2),
    request('ex-3', 'turn-1', start + 95_000, 31800, 23500, 2260, 0), request('ex-4', 'turn-2', start + 600_000, 38900, 31200, 1150, 4),
    request('ex-5', 'turn-2', start + 660_000, 52600, 37400, 640, 0, 'compactor', 'gpt-5.6-luna'), request('ex-6', 'turn-2', start + 700_000, 21300, 0, 1830, 1),
  ] };
  const quota = { observed_at: now(), plan: 'Pro', credits: null, buckets: [{ id: 'codex', name: 'Codex', allowed: true, limit_reached: false, windows: [
    { id: 'primary', used_percent: 38, duration_seconds: 18000, resets_at: now() + 8220 },
    { id: 'secondary', used_percent: 64, duration_seconds: 604800, resets_at: now() + 262800 },
  ] }] };
  const config = { profile: 'demo', registered_tools: ['read_file', 'list_dir', 'grep', 'apply_patch', 'shell', 'update_plan'].map(name => ({ name })) };
  return {
    'agent.session.read': sessionScenario(signal),
    'agent.workspace.read': view => Object.freeze({
      list: path => { view.throwIfAborted(); try { return Promise.resolve(listing(path)); } catch (error) { return Promise.reject(error); } },
      read: path => FILES[path] === undefined ? Promise.reject(new Error('Файл не найден')) : reply(view, { path, size: FILES[path].length, kind: 'text', text: FILES[path] }),
      changes: () => reply(view, { repository: true, truncated: false, entries: Object.entries(CHANGES).map(([path, status]) => ({ path, status })) }),
      diff: path => reply(view, { path, kind: 'text', patch: PATCHES[path] ?? '' }),
    }),
    'agent.model.quota.read': view => Object.freeze({ read: () => reply(view, quota) }),
    'agent.usage.read': view => Object.freeze({ read: () => reply(view, usage) }),
    'agent.config.read': view => Object.freeze({ read: () => reply(view, config) }),
    'client.preferences': stateService({ fontSize: 16, chatWidth: 820, animations: true, autoScroll: true, toolCardsCollapsed: true, notifications: false, sendMode: 'enter' },
      ['fontSize', 'chatWidth', 'animations', 'autoScroll', 'toolCardsCollapsed', 'notifications', 'sendMode']),
    'client.composer': stateService({
      model: 'gpt-5.6', models: [{ name: 'gpt-5.6', label: 'GPT-5.6', hidden: false }, { name: 'gpt-5.6-luna', label: 'GPT-5.6 Luna', hidden: false }],
      reasoning: true, effort: 'medium', effortLabel: 'Medium', efforts: ['low', 'medium', 'high'], mode: 'normal',
      modes: [{ value: 'normal', label: 'По правилам', description: 'Решает политика подтверждений профиля.' },
        { value: 'auto', label: 'Правки без вопросов', description: 'Файлы меняются без подтверждения; команды и сеть запрещены.' },
        { value: 'plan', label: 'Планирование', description: 'Только чтение: агент изучает проект и предлагает план.' }],
    }, ['model', 'effort', 'mode']),
  };
}
