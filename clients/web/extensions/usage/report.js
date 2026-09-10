import { readRates } from './pricing.js';
import { node, renderSummary, renderModels, renderRequests, priceNote } from './render.js';
import { style } from './style.js';

export function mountReport({ root, services, storage, signal }, wide = false) {
  root.append(node('style', style));
  const surface = node('div', null, wide ? 'wide' : 'compact');
  const controls = node('div', null, 'usage-controls');
  const scope = node('select'); scope.setAttribute('aria-label', 'Период расхода');
  const model = node('select'); model.setAttribute('aria-label', 'Модель для отчёта');
  controls.append(scope, model);
  const content = node('div');
  const status = node('span', 'Загрузка…', 'muted'); status.setAttribute('role', 'status');
  const refresh = node('button', 'Обновить'); refresh.type = 'button';
  const footer = node('div', null, 'usage-footer'); footer.append(status, refresh);
  surface.append(controls, content, footer, priceNote()); root.append(surface);
  let snapshot, pending = false, pricingChanged = false, page = 0, scopeValue = 'all', modelValue = 'all';
  let rowsController;
  function options(select, items, value) {
    select.replaceChildren(...items.map(([id, label]) => { const option = node('option', label); option.value = id; return option; }));
    select.value = items.some(item => item[0] === value) ? value : 'all';
  }
  function render() {
    rowsController?.abort(); rowsController = new AbortController();
    if (!snapshot) { content.replaceChildren(node('p', 'Журнал расхода недоступен для этого чата.', 'muted')); controls.hidden = true; return; }
    controls.hidden = false;
    const rates = readRates(storage);
    const opened = new Set([...content.querySelectorAll('.usage-request[open]')].map(item => item.dataset.exchangeId));
    const modelsOpen = content.querySelector('.usage-models')?.open;
    const recentOpen = content.querySelector('.usage-recent')?.open;
    const turns = [...new Set(snapshot.requests.map(item => item.turn_id).filter(Boolean))];
    options(scope, [['all', 'Весь чат'], ['latest', 'Последний ход'], ...(wide ? turns.map((id, index) => [id, `Ход ${index + 1}`]).reverse() : [])], scopeValue);
    const models = [...new Set(snapshot.requests.map(item => JSON.stringify([item.model.provider, item.model.model])))];
    options(model, [['all', 'Все модели'], ...models.map(id => [id, JSON.parse(id).join(' / ')])], modelValue);
    model.hidden = models.length < 2;
    const selectedTurn = scope.value === 'latest' ? snapshot.latest_turn_id : scope.value;
    const requests = snapshot.requests.filter(item => (scope.value === 'all' || (selectedTurn !== null && item.turn_id === selectedTurn)) && (model.value === 'all' || JSON.stringify([item.model.provider, item.model.model]) === model.value));
    const groups = renderModels(requests, rates); groups.open = !!modelsOpen;
    const list = renderRequests(requests, snapshot.requests, rates, wide, page, opened); page = list.current;
    const pages = node('div', null, 'pagination');
    const previous = node('button', 'Назад'), next = node('button', 'Дальше');
    previous.type = next.type = 'button'; previous.disabled = page === 0; next.disabled = page === list.last;
    previous.addEventListener('click', () => { page--; render(); }, { signal: rowsController.signal });
    next.addEventListener('click', () => { page++; render(); }, { signal: rowsController.signal });
    pages.append(previous, node('span', `${page + 1} / ${list.last + 1}`), next); pages.hidden = list.last === 0;
    if (wide) content.replaceChildren(renderSummary(requests, rates), groups, list.box, pages);
    else {
      const recent = node('details', null, 'usage-recent'); recent.open = !!recentOpen;
      recent.append(node('summary', `Запросы · ${requests.length}`), list.box, pages);
      content.replaceChildren(renderSummary(requests, rates), groups, recent);
    }
  }
  async function update(force = false) {
    if (pending || signal.aborted) return;
    pending = true; refresh.disabled = true;
    try {
      const next = await services['agent.usage.read'].read();
      if (signal.aborted) return;
      const changed = force || pricingChanged || next?.session_id !== snapshot?.session_id || next?.revision !== snapshot?.revision || !content.firstChild;
      snapshot = next;
      if (changed) { render(); pricingChanged = false; }
      status.className = 'muted'; status.textContent = `Обновлено ${new Date().toLocaleTimeString('ru-RU')}`;
    } catch (error) {
      if (signal.aborted) return;
      snapshot = undefined; content.replaceChildren();
      status.className = 'error'; status.textContent = `Не удалось получить расход: ${error.message}`;
    } finally { pending = false; if (!signal.aborted) refresh.disabled = false; }
  }
  scope.addEventListener('change', () => { scopeValue = scope.value; page = 0; render(); }, { signal });
  model.addEventListener('change', () => { modelValue = model.value; page = 0; render(); }, { signal });
  refresh.addEventListener('click', () => void update(true), { signal });
  const unsubscribe = storage.subscribe(() => { pricingChanged = true; void update(true); });
  const timer = setInterval(() => void update(), 5000);
  const stop = () => { clearInterval(timer); rowsController?.abort(); unsubscribe(); };
  signal.addEventListener('abort', stop, { once: true });
  void update();
  return stop;
}
