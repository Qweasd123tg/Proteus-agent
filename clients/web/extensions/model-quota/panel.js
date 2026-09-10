import { duration, percent, remaining, resetLabel, timestamp } from './format.js';

function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text != null) node.textContent = text;
  if (className) node.className = className;
  return node;
}

function render(snapshot, content) {
  if (snapshot === null) {
    content.append(element('p', 'Текущий провайдер не предоставляет сведения о лимитах.', 'muted'));
    return;
  }
  if (snapshot.plan) content.append(element('p', `План · ${snapshot.plan}`, 'muted'));
  if (!snapshot.buckets.length) content.append(element('p', 'Окна лимитов не предоставлены.', 'muted'));
  for (const bucket of snapshot.buckets) {
    const section = element('section', null, 'bucket');
    section.append(element('h3', bucket.name || bucket.id));
    if (bucket.limit_reached === true) section.append(element('p', 'Лимит исчерпан', 'error'));
    else if (bucket.allowed === false) section.append(element('p', 'Использование сейчас недоступно', 'error'));
    if (!bucket.windows.length) section.append(element('p', 'Окна лимитов не предоставлены.', 'muted'));
    for (const window of bucket.windows) {
      const left = remaining(window.used_percent);
      const row = element('div', null, 'row');
      row.append(element('span', duration(window.duration_seconds, window.id)), element('strong', `${percent(left)} осталось`));
      const bar = element('progress');
      bar.max = 100;
      bar.value = left;
      bar.setAttribute('aria-label', `${bucket.name || bucket.id}: ${duration(window.duration_seconds, window.id)}, остаток`);
      if (left <= 10) bar.className = 'low';
      section.append(row, bar, element('p', resetLabel(window.resets_at), 'muted reset'));
    }
    content.append(section);
  }
  if (snapshot.credits) {
    const credits = snapshot.credits;
    const value = credits.unlimited ? 'Без ограничений' : (credits.balance ?? (credits.available ? 'Доступны' : 'Нет доступных'));
    const row = element('div', null, 'row');
    row.append(element('span', 'Кредиты'), element('strong', value));
    content.append(row);
  }
}

export function mount({ root, services, signal }) {
  const style = element('style', `
    h3 { font-size: 13px; font-weight: 500; margin: 0 0 8px; overflow-wrap: anywhere; }
    .bucket + .bucket { margin-top: 16px; padding-top: 12px; border-top: 1px solid var(--border-subtle, #333); }
    progress { width: 100%; height: 5px; display: block; border: 0; border-radius: 4px; overflow: hidden; background: var(--bg-panel-soft, #333); accent-color: var(--text-muted, #a0a0a0); }
    progress::-moz-progress-bar { background: var(--text-muted, #a0a0a0); }
    progress::-webkit-progress-bar { background: var(--bg-panel-soft, #333); }
    progress::-webkit-progress-value { background: var(--text-muted, #a0a0a0); }
    progress.low::-moz-progress-bar { background: var(--accent-red, #ef6b6b); }
    progress.low::-webkit-progress-value { background: var(--accent-red, #ef6b6b); }
    .reset { font-size: 11px; margin: 5px 0 12px; }
    .footer { margin-top: 12px; display: flex; align-items: center; gap: 8px; justify-content: space-between; }
    .footer p { margin: 0; font-size: 11px; }
  `);
  const content = element('div');
  const status = element('p', '', 'muted');
  status.setAttribute('role', 'status');
  const refresh = element('button', 'Обновить');
  refresh.type = 'button';
  const footer = element('div', null, 'footer');
  footer.append(status, refresh);
  root.append(style, content, footer);
  let pending = false;
  async function update() {
    if (pending || signal.aborted) return;
    pending = true;
    refresh.disabled = true;
    status.className = 'muted';
    status.textContent = 'Загрузка…';
    try {
      const snapshot = await services['agent.model.quota.read'].read();
      if (signal.aborted) return;
      content.replaceChildren();
      render(snapshot, content);
      status.textContent = snapshot === null ? '' : `Данные на ${timestamp(snapshot.observed_at)}`;
    } catch (error) {
      if (signal.aborted) return;
      content.replaceChildren();
      status.className = 'error';
      status.textContent = `Не удалось получить лимиты: ${error.message}`;
    } finally {
      pending = false;
      if (!signal.aborted) refresh.disabled = false;
    }
  }
  refresh.addEventListener('click', update, { signal });
  const timer = setInterval(update, 60_000);
  signal.addEventListener('abort', () => clearInterval(timer), { once: true });
  void update();
  return () => clearInterval(timer);
}
