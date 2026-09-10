import { parseManifest, parseSettings, resourceUrl } from './contract.js';
import { settingsStore } from './storage.js';
import { button, createPanel } from './panel.js';

async function readJson(url, signal) {
  const response = await fetch(url, { signal, credentials: 'omit', cache: 'no-store' });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return { value: await response.json(), url: response.url };
}

// Client-owned registry: интерфейсы предоставляются оболочкой явно. Сам
// загрузчик не знает transport, credentials или types конкретного агента.
export function mountExtensions(root, services = {}, options = {}) {
  const controller = new AbortController();
  const { signal } = controller;
  const catalogUrl = new URL('./catalog.json', import.meta.url).href;
  const storage = options.storage ?? {
    getItem: key => localStorage.getItem(key),
    setItem: (key, value) => localStorage.setItem(key, value),
    removeItem: key => localStorage.removeItem(key),
  };
  const store = settingsStore(storage);
  const cards = new Map();
  let records = [];
  let bundled = [];
  let initialized = false;
  let busy = false;
  let listController = new AbortController();

  const manager = document.createElement('details');
  manager.className = 'extension-manager';
  const summary = document.createElement('summary');
  summary.textContent = 'Расширения';
  const list = document.createElement('div');
  list.className = 'extension-list';
  const available = document.createElement('div');
  available.className = 'extension-available';
  const form = document.createElement('form');
  form.className = 'extension-install';
  const input = document.createElement('input');
  input.type = 'url';
  input.required = true;
  input.placeholder = 'URL манифеста расширения';
  input.setAttribute('aria-label', 'URL манифеста расширения');
  const submit = document.createElement('button');
  submit.type = 'submit';
  submit.textContent = 'Добавить';
  const notice = document.createElement('p');
  notice.className = 'extension-error';
  notice.setAttribute('role', 'status');
  const reset = button('Восстановить список поставляемых расширений', () => initialize(true), signal);
  reset.className = 'extension-reset';
  form.append(input, submit);
  manager.append(summary, list, available, form, notice, reset);
  const panels = document.createElement('div');
  panels.className = 'extension-panels';
  root.append(manager, panels);

  function save() {
    try {
      store.write(records.map(({ id, url, enabled, collapsed }) => ({ id, url, enabled, collapsed })));
      notice.textContent = '';
    } catch { notice.textContent = 'Изменения действуют до закрытия: не удалось сохранить настройки.'; }
  }

  function reconcile() {
    for (const [id, card] of cards) {
      if (!records.some(record => record.id === id && record.enabled)) { card.stop(); cards.delete(id); }
    }
    for (const record of records.filter(record => record.enabled)) {
      if (!cards.has(record.id)) {
        cards.set(record.id, createPanel(record, { services, storage, signal, changed: () => { save(); reconcile(); } }));
      }
      const card = cards.get(record.id);
      card.update();
      panels.append(card.element);
    }
    renderList();
  }

  function renderList() {
    listController.abort();
    listController = new AbortController();
    const listSignal = listController.signal;
    list.replaceChildren();
    records.forEach((record, index) => {
      const row = document.createElement('div');
      row.className = 'extension-choice';
      row.dataset.extensionChoice = record.id;
      const label = document.createElement('label');
      const checkbox = document.createElement('input');
      checkbox.type = 'checkbox';
      checkbox.checked = record.enabled;
      checkbox.addEventListener('change', () => { record.enabled = checkbox.checked; save(); reconcile(); }, { signal: listSignal });
      const text = document.createElement('span');
      text.textContent = record.manifest?.name ?? record.id;
      text.title = record.error ?? record.manifest.description;
      label.append(checkbox, text);
      row.append(label);
      for (const [step, glyph, action] of [[-1, '↑', 'Выше'], [1, '↓', 'Ниже']]) {
        const move = button(glyph, () => {
          [records[index], records[index + step]] = [records[index + step], records[index]];
          save(); reconcile();
        }, listSignal);
        move.disabled = index + step < 0 || index + step >= records.length;
        move.setAttribute('aria-label', `${action}: ${text.textContent}`);
        row.append(move);
      }
      const remove = button('×', () => { records = records.filter(item => item !== record); save(); reconcile(); }, listSignal);
      remove.setAttribute('aria-label', `Убрать: ${text.textContent}`);
      row.append(remove);
      list.append(row);
    });
    available.replaceChildren();
    for (const record of bundled.filter(item => !records.some(current => current.id === item.id))) {
      const add = button(`Добавить: ${record.manifest?.name ?? record.id}`, () => {
        if (busy || !initialized) return;
        records.push({ ...record, enabled: true, collapsed: false });
        save(); reconcile();
      }, listSignal);
      add.dataset.extensionAvailable = record.id;
      add.disabled = !!record.error;
      if (record.error) add.title = record.error;
      available.append(add);
    }
  }

  async function resolveRecord(record) {
    try {
      const response = await readJson(record.url, signal);
      const manifest = parseManifest(response.value, response.url);
      if (manifest.id !== record.id) throw new Error(`id манифеста изменился: ${manifest.id}`);
      return { ...record, manifest };
    } catch (error) { return { ...record, error: `Манифест: ${error.message}` }; }
  }

  async function initialize(defaults = false) {
    if (busy) return;
    busy = true;
    submit.disabled = true;
    try {
      const saved = defaults ? null : store.read();
      let catalogError = '';
      try {
        const catalog = (await readJson(catalogUrl, signal)).value;
        bundled = await Promise.all(parseSettings(catalog, catalogUrl).map(resolveRecord));
      } catch (error) {
        if (saved === null) throw error;
        bundled = [];
        catalogError = `Не удалось загрузить список поставляемых расширений: ${error.message}`;
      }
      const next = saved === null ? bundled.map(record => ({ ...record }))
        : await Promise.all(parseSettings(JSON.parse(saved), catalogUrl).map(resolveRecord));
      if (signal.aborted) return;
      for (const card of cards.values()) card.stop();
      cards.clear();
      records = next;
      initialized = true;
      notice.textContent = catalogError;
      if (defaults) save();
      reconcile();
    } catch (error) {
      if (!signal.aborted) { notice.textContent = `Не удалось загрузить расширения: ${error.message}`; manager.open = true; }
    } finally { busy = false; submit.disabled = !initialized; }
  }

  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (busy || !initialized) return;
    busy = true;
    submit.disabled = true;
    try {
      const url = resourceUrl(input.value, catalogUrl);
      const response = await readJson(url, signal);
      const manifest = parseManifest(response.value, response.url);
      if (signal.aborted) return;
      if (records.some(record => record.id === manifest.id)) throw new Error(`Расширение ${manifest.id} уже добавлено`);
      records.push({ id: manifest.id, url, manifest, enabled: true, collapsed: false });
      save(); reconcile(); input.value = '';
    } catch (error) { if (!signal.aborted) notice.textContent = `Не удалось добавить расширение: ${error.message}`; }
    finally { busy = false; submit.disabled = false; }
  }, { signal });

  void initialize();
  return () => {
    controller.abort();
    listController.abort();
    for (const card of cards.values()) card.stop();
    cards.clear();
    root.replaceChildren();
  };
}
