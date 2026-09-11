import { icon } from '../dom.js';
export function mount({ root, compact, services, signal }) {
  icon(compact, 'M4 4h6v6H4ZM14 4h6v6h-6ZM4 14h6v6H4ZM14 14h6v6h-6Z');
  const content = document.createElement('div');
  const status = document.createElement('p');
  status.className = 'muted';
  status.setAttribute('role', 'status');
  const refresh = document.createElement('button');
  refresh.textContent = 'Обновить';
  root.append(content, status, refresh);
  function row(label, value) {
    const line = document.createElement('div');
    line.className = 'row';
    const key = document.createElement('span');
    key.textContent = label;
    const text = document.createElement('strong');
    text.textContent = value;
    line.append(key, text);
    content.append(line);
  }
  async function update() {
    refresh.disabled = true;
    status.className = 'muted';
    status.textContent = 'Подключение…';
    try {
      const config = await services['agent.config.read'].read();
      if (signal.aborted) return;
      if (typeof config.profile !== 'string' || !Array.isArray(config.registered_tools)) throw new Error('Неизвестный формат сведений об агенте');
      content.replaceChildren();
      row('Профиль', config.profile);
      row('Инструменты', String(config.registered_tools.length));
      const tools = document.createElement('details');
      const label = document.createElement('summary');
      label.textContent = 'Доступные инструменты';
      tools.append(label);
      for (const tool of config.registered_tools) {
        const name = document.createElement('div');
        name.textContent = tool.name;
        tools.append(name);
      }
      content.append(tools);
      status.textContent = `Обновлено в ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
    } catch (error) {
      if (signal.aborted) return;
      content.replaceChildren();
      status.className = 'error';
      status.textContent = `Нет данных: ${error.message}`;
    } finally {
      if (!signal.aborted) refresh.disabled = false;
    }
  }
  refresh.addEventListener('click', update, { signal });
  void update();
}
