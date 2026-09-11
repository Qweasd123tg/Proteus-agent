import { createPanelRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { theme } from './theme.js';

export function button(label, action, signal) {
  const element = document.createElement('button');
  element.type = 'button';
  element.textContent = label;
  element.addEventListener('click', action, { signal });
  return element;
}

export function createPanel(record, { services, storage, changed }) {
  const controller = new AbortController();
  const { signal } = controller;
  const element = document.createElement('section');
  element.className = 'extension-panel';
  element.dataset.extensionId = record.id;
  const header = document.createElement('div');
  header.className = 'extension-panel-header';
  const title = button(record.manifest?.name ?? record.id, () => {
    record.collapsed = !record.collapsed;
    changed();
  }, signal);
  title.className = 'extension-panel-title';
  const name = document.createElement('span');
  name.textContent = title.textContent;
  const toggle = document.createElement('span');
  toggle.className = 'extension-panel-toggle';
  toggle.setAttribute('aria-hidden', 'true');
  title.replaceChildren(name, toggle);
  const body = document.createElement('div');
  body.className = 'extension-panel-body';
  body.id = `extension-body-${record.id}`;
  title.setAttribute('aria-controls', body.id);
  const error = document.createElement('p');
  error.className = 'extension-error';
  error.setAttribute('role', 'status');
  const retry = button('Повторить', () => mount(), signal);
  retry.hidden = true;
  header.append(title);
  element.append(header, body, error, retry);
  let runtime;

  function mount() {
    runtime?.stop();
    // Новый root для каждого mount: поздняя очистка отменённой async-панели
    // может затронуть только отсоединённое дерево, а не следующую инстанцию.
    const surface = document.createElement('div');
    surface.className = 'extension-panel-content';
    body.replaceChildren(surface);
    const shadow = surface.attachShadow({ mode: 'open' });
    error.textContent = '';
    retry.hidden = true;
    if (record.error) {
      error.textContent = record.error;
      return;
    }
    const style = document.createElement('style');
    style.textContent = theme;
    shadow.append(style);
    runtime = createPanelRuntime({
      manifest: record.manifest, root: shadow, services,
      storage: extensionStorage(storage, record.id),
      onError(failure) {
        shadow.replaceChildren();
        error.textContent = `Не удалось открыть панель: ${failure.message}`;
        retry.hidden = false;
      },
    });
  }

  let expanded;
  function update() {
    title.setAttribute('aria-expanded', String(!record.collapsed));
    title.title = record.collapsed ? 'Развернуть панель' : 'Свернуть панель';
    toggle.textContent = record.collapsed ? '+' : '−';
    body.hidden = record.collapsed;
    error.hidden = record.collapsed;
    if (expanded === !record.collapsed) return;
    expanded = !record.collapsed;
    if (expanded) mount();
    else { runtime?.stop(); runtime = undefined; retry.hidden = true; }
  }
  update();
  return { element, update, stop() { controller.abort(); runtime?.stop(); element.remove(); } };
}
