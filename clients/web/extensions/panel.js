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

export function createPanel(record, { services, storage, changed, onOpen }) {
  const controller = new AbortController();
  const { signal } = controller;
  const element = document.createElement('section');
  element.className = 'extension-panel';
  element.dataset.extensionId = record.id;
  const header = document.createElement('div');
  header.className = 'extension-panel-header';
  const title = button(record.manifest?.name ?? record.id, () => {
    record.collapsed = !record.collapsed;
    changed({ collapsed: record.collapsed });
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
  const compactButton = button('', () => {
    changed({ collapsed: false });
    onOpen?.(record.location);
  }, signal);
  compactButton.className = 'extension-compact';
  compactButton.title = record.manifest?.name ?? record.id;
  compactButton.setAttribute('aria-label', compactButton.title);
  const compactSurface = document.createElement('span'); compactButton.append(compactSurface);
  const compact = compactSurface.attachShadow({ mode: 'open' });
  const compactStyle = document.createElement('style');
  compactStyle.textContent = ':host{display:grid;place-items:center;color:inherit;font:inherit}svg{width:28px;height:28px}';
  compact.append(compactStyle, document.createTextNode((record.manifest?.name ?? record.id).slice(0, 1)));
  const placement = document.createElement('select');
  placement.className = 'extension-placement';
  placement.setAttribute('aria-label', `Область: ${record.manifest?.name ?? record.id}`);
  for (const [value, label] of [['left', 'Слева'], ['right', 'Справа'], ['main', 'В центре']]) {
    const option = document.createElement('option'); option.value = value; option.textContent = label; placement.append(option);
  }
  placement.addEventListener('change', () => changed({ location: placement.value, collapsed: false }), { signal });
  header.append(compactButton, title, placement);
  element.append(header, body, error, retry);
  let runtime;
  let failed = false;

  function mount() {
    runtime?.stop();
    // Новый root для каждого mount: поздняя очистка отменённой async-панели
    // может затронуть только отсоединённое дерево, а не следующую инстанцию.
    const surface = document.createElement('div');
    surface.className = 'extension-panel-content';
    body.replaceChildren(surface);
    const shadow = surface.attachShadow({ mode: 'open' });
    failed = false;
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
      manifest: record.manifest, root: shadow, compact,
      panel: Object.freeze({ open: () => compactButton.click(), move: location => changed({ location, collapsed: false }) }), services,
      storage: extensionStorage(storage, record.id),
      onError(failure) {
        shadow.replaceChildren();
        error.textContent = `Не удалось открыть панель: ${failure.message}`;
        failed = true;
        retry.hidden = record.collapsed;
      },
    });
  }

  function update() {
    title.setAttribute('aria-expanded', String(!record.collapsed));
    title.title = record.collapsed ? 'Развернуть панель' : 'Свернуть панель';
    toggle.textContent = record.collapsed ? '+' : '−';
    body.hidden = record.collapsed;
    element.classList.toggle('expanded', !record.collapsed);
    placement.value = record.location;
    placement.title = 'Переместить панель';
    error.hidden = record.collapsed;
    retry.hidden = record.collapsed || !failed;
  }
  // The compact surface needs the same live instance even when initially collapsed.
  mount();
  update();
  return { element, update, stop() { controller.abort(); runtime?.stop(); element.remove(); } };
}
