import { button } from './panel.js';
import { mountSettingsEntry } from './settings-entry.js';

function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text) element.textContent = text;
  if (className) element.className = className;
  return element;
}

export function mountExtensionSettings(root, registry, services = {}) {
  const controller = new AbortController();
  const { signal } = controller;
  let rowsController;
  let closeOptions, optionsId;
  const options = node('section', '', 'extension-options'); options.hidden = true;
  const optionsTitle = node('h3');
  const optionsBody = node('div');
  function close() { closeOptions?.(); closeOptions = undefined; optionsId = undefined; options.hidden = true; }
  options.append(button('Закрыть настройки панели', close, signal), optionsTitle, optionsBody);
  const list = node('div', '', 'extension-list');
  const available = node('div', '', 'extension-available');
  const notice = node('p', '', 'extension-error');
  notice.setAttribute('role', 'status');
  const source = node('details', '', 'extension-source');
  source.append(node('summary', 'Добавить по ссылке'));
  const form = node('form', '', 'extension-install');
  const input = node('input');
  input.type = 'url'; input.required = true;
  input.placeholder = 'https://example.com/extension.json';
  input.setAttribute('aria-label', 'URL манифеста расширения');
  const submit = node('button', 'Добавить'); submit.type = 'submit';
  form.append(input, submit); source.append(form);
  const reset = node('details', '', 'extension-reset');
  reset.append(node('summary', 'Восстановить стандартный список'));
  reset.append(node('p', 'Состав и порядок панелей заменятся поставляемым списком. Заметки сохранятся.', 'settings-hint'));
  const restore = button('Восстановить', () => { reset.open = false; void registry.reset(); }, signal);
  reset.append(restore);
  root.append(list, options, available, source, notice, reset);
  const unsubscribe = registry.subscribe(() => {
    const { records, bundled, notice: message, busy, ready } = registry.state();
    if (optionsId && !records.some(record => record.id === optionsId)) close();
    const focusKey = document.activeElement?.dataset.controlKey;
    rowsController?.abort(); rowsController = new AbortController();
    const rowSignal = rowsController.signal;
    notice.textContent = message || (!ready && busy ? 'Загрузка расширений…' : '');
    submit.disabled = busy || !ready; restore.disabled = busy;
    list.replaceChildren(); available.replaceChildren();
    if (ready && !records.length) list.append(node('p', 'Панелей пока нет. Добавьте одну из доступных ниже.', 'settings-hint'));
    records.forEach((record, index) => {
      const row = node('div', '', 'extension-choice'); row.dataset.extensionChoice = record.id;
      const label = node('label', '', 'extension-description');
      const text = node('span');
      const name = record.manifest?.name ?? record.id;
      text.append(node('strong', name), node('span', record.error ?? record.manifest?.description, record.error ? 'extension-error' : 'settings-hint'));
      const checkbox = node('input'); checkbox.type = 'checkbox'; checkbox.checked = record.enabled;
      checkbox.className = 'settings-toggle'; checkbox.disabled = busy; checkbox.dataset.controlKey = record.id;
      checkbox.addEventListener('change', () => registry.update(record.id, { enabled: checkbox.checked }), { signal: rowSignal });
      label.append(text, checkbox); row.append(label);
      const actions = node('div', '', 'extension-actions');
      if (record.manifest?.settings) {
        const configure = button('Настроить', () => {
          close(); optionsId = record.id; options.hidden = false; optionsTitle.textContent = name;
          closeOptions = mountSettingsEntry(optionsBody, record, registry.storage, services);
          options.scrollIntoView({ block: 'nearest' });
        }, rowSignal);
        configure.disabled = busy; configure.setAttribute('aria-label', `Настроить: ${name}`);
        actions.append(configure);
      }
      for (const [step, glyph, action] of [[-1, '↑', 'Выше'], [1, '↓', 'Ниже']]) {
        const move = button(glyph, () => registry.move(record.id, step), rowSignal);
        move.disabled = busy || index + step < 0 || index + step >= records.length;
        move.setAttribute('aria-label', `${action}: ${name}`); move.title = `${action}: ${name}`;
        move.dataset.controlKey = `${record.id}:${step}`; actions.append(move);
      }
      const remove = button('Убрать', () => registry.remove(record.id), rowSignal);
      remove.disabled = busy; remove.setAttribute('aria-label', `Убрать: ${name}`);
      actions.append(remove); row.append(actions); list.append(row);
    });
    const choices = bundled.filter(item => !records.some(record => record.id === item.id));
    if (choices.length) available.append(node('h3', 'Доступные панели'));
    for (const record of choices) {
      const row = node('div', '', 'extension-choice');
      const description = node('div', '', 'extension-description');
      const text = node('span');
      text.append(node('strong', record.manifest?.name ?? record.id), node('span', record.error ?? record.manifest?.description, 'settings-hint'));
      description.append(text); row.append(description);
      const add = button('Добавить', () => registry.addBundled(record.id), rowSignal);
      add.dataset.extensionAvailable = record.id; add.disabled = busy || !ready || !!record.error;
      add.setAttribute('aria-label', `Добавить: ${record.manifest?.name ?? record.id}`);
      row.append(add); available.append(row);
    }
    if (focusKey) [...root.querySelectorAll('[data-control-key]')].find(item => item.dataset.controlKey === focusKey)?.focus();
  });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (await registry.install(input.value)) input.value = '';
  }, { signal });
  void registry.start();
  return () => { close(); controller.abort(); rowsController?.abort(); unsubscribe(); root.replaceChildren(); };
}
