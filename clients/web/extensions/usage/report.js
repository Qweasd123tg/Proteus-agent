import { readRates } from './pricing.js';
import { node, renderSummary, renderModels, renderRequests, priceNote } from './render.js';
import { modelKey, turnIds, scopedRequests, selectRequests, requestExport } from './selection.js';
import { style } from './style.js';

function options(select, items, value) {
  select.replaceChildren(...items.map(([id, label]) => { const option = node('option', label); option.value = id; return option; }));
  select.value = items.some(item => item[0] === value) ? value : 'all';
}

function selector(label, items, value) {
  const select = node('select'); select.setAttribute('aria-label', label);
  options(select, items, value);
  return select;
}

export function mountReport({ root, services, storage, signal }, wide = false) {
  root.append(node('style', style));
  const surface = node('div', null, wide ? 'wide' : 'compact');
  const controls = node('div', null, 'usage-controls');
  const scope = selector('Период расхода', [['all', 'Весь чат']], 'all');
  const model = selector('Модель для отчёта', [['all', 'Все модели']], 'all');
  controls.append(scope, model);
  const overview = node('div');
  const filters = node('div', null, 'usage-filters');
  const state = selector('Статус запроса', [['all', 'Все статусы'], ['problems', 'Проблемные запросы'], ['error', 'Ошибки'], ['canceled', 'Отмена'], ['timeout', 'Таймаут'], ['unfinished', 'Нет результата'], ['missing_usage', 'Нет данных о токенах'], ['completed', 'Завершённые']], 'all');
  const origin = selector('Тип запроса', [['all', 'Все типы'], ['direct', 'Запрос модели'], ['compactor', 'Сжатие контекста']], 'all');
  const order = selector('Порядок запросов', [['oldest', 'По порядку выполнения'], ['newest', 'Сначала последние'], ['duration', 'Сначала долгие'], ['tokens', 'Сначала больше токенов'], ['cost', 'Сначала дороже']], 'oldest');
  const search = node('input'); search.type = 'search'; search.placeholder = 'ID запроса, хода или модель'; search.setAttribute('aria-label', 'Поиск запросов');
  const reset = node('button', 'Сбросить фильтры'); reset.type = 'button';
  filters.append(search, state, origin, order, reset);
  const listHeading = node('div', null, 'usage-list-heading');
  const count = node('span'); count.setAttribute('role', 'status');
  const download = node('button', 'Скачать JSON выборки'); download.type = 'button';
  listHeading.append(count, download);
  const content = node('div');
  const status = node('span', 'Загрузка…', 'muted'); status.setAttribute('role', 'status');
  const refresh = node('button', 'Обновить'); refresh.type = 'button';
  const footer = node('div', null, 'usage-footer'); footer.append(status, refresh);
  surface.append(controls, overview);
  if (wide) surface.append(filters, listHeading);
  surface.append(content, footer, priceNote()); root.append(surface);
  let snapshot, pending = false, pricingChanged = false, page = 0, scopeValue = 'all', modelValue = 'all';
  let rowsController, exportRows = [];
  const diagnosticFilters = () => ({ status: state.value, origin: origin.value, query: search.value, sort: order.value });

  function render() {
    rowsController?.abort(); rowsController = new AbortController();
    controls.hidden = filters.hidden = listHeading.hidden = !snapshot;
    download.disabled = !snapshot;
    if (!snapshot) {
      overview.replaceChildren();
      content.replaceChildren(node('p', 'Журнал расхода недоступен для этого чата.', 'muted'));
      exportRows = [];
      return;
    }
    const rates = readRates(storage);
    const opened = new Set([...content.querySelectorAll('.usage-request[open]')].map(item => item.dataset.exchangeId));
    const modelsOpen = overview.querySelector('.usage-models')?.open;
    const recentOpen = content.querySelector('.usage-recent')?.open;
    options(scope, [['all', 'Весь чат'], ['latest', 'Последний ход'], ...(wide ? turnIds(snapshot.requests).map((id, index) => [id, `Ход ${index + 1}`]).reverse() : [])], scopeValue);
    const models = [...new Set(snapshot.requests.map(modelKey))];
    options(model, [['all', 'Все модели'], ...models.map(id => [id, JSON.parse(id).join(' / ')])], modelValue);
    model.hidden = models.length < 2;
    const scoped = scopedRequests(snapshot, scope.value, model.value);
    exportRows = wide ? selectRequests(scoped, diagnosticFilters(), rates) : scoped;
    const groups = renderModels(scoped, rates); groups.open = !!modelsOpen;
    overview.replaceChildren(...(wide ? [node('h3', 'Итог выбранного периода', 'usage-scope-heading')] : []), renderSummary(scoped, rates), groups);
    const list = renderRequests(exportRows, snapshot.requests, rates, wide, page, opened); page = list.current;
    count.textContent = `Запросы: ${exportRows.length} из ${scoped.length} · итог выше относится ко всему выбранному периоду`;
    download.disabled = !exportRows.length;
    reset.disabled = state.value === 'all' && origin.value === 'all' && !search.value && order.value === 'oldest';
    const pages = node('div', null, 'pagination');
    const previous = node('button', 'Назад'), next = node('button', 'Дальше');
    previous.type = next.type = 'button'; previous.disabled = page === 0; next.disabled = page === list.last;
    previous.addEventListener('click', () => { page--; render(); }, { signal: rowsController.signal });
    next.addEventListener('click', () => { page++; render(); }, { signal: rowsController.signal });
    pages.append(previous, node('span', `${page + 1} / ${list.last + 1}`), next); pages.hidden = list.last === 0;
    if (wide) content.replaceChildren(list.box, pages);
    else {
      const recent = node('details', null, 'usage-recent'); recent.open = !!recentOpen;
      recent.append(node('summary', `Запросы · ${scoped.length}`), list.box, pages);
      content.replaceChildren(recent);
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
      snapshot = undefined; render();
      status.className = 'error'; status.textContent = `Не удалось получить расход: ${error.message}`;
    } finally { pending = false; if (!signal.aborted) refresh.disabled = false; }
  }
  const change = () => { page = 0; render(); };
  scope.addEventListener('change', () => { scopeValue = scope.value; change(); }, { signal });
  model.addEventListener('change', () => { modelValue = model.value; change(); }, { signal });
  for (const control of [state, origin, order]) control.addEventListener('change', change, { signal });
  search.addEventListener('input', change, { signal });
  reset.addEventListener('click', () => { state.value = origin.value = 'all'; search.value = ''; order.value = 'oldest'; change(); }, { signal });
  download.addEventListener('click', () => {
    if (!snapshot || !exportRows.length) return;
    const json = requestExport(snapshot, exportRows, { scope: scope.value, model: model.value, ...diagnosticFilters() });
    const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
    const link = node('a'); link.href = url; link.download = 'proteus-session-requests.json';
    surface.append(link); link.click(); link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }, { signal });
  refresh.addEventListener('click', () => void update(true), { signal });
  const unsubscribe = storage.subscribe(() => { pricingChanged = true; void update(true); });
  const timer = setInterval(() => void update(), 5000);
  const stop = () => { clearInterval(timer); rowsController?.abort(); unsubscribe(); };
  signal.addEventListener('abort', stop, { once: true });
  void update();
  return stop;
}
