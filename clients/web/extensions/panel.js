import { createPanelRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { theme } from './theme.js';
import { icon } from './icons.js';
import { attachPanelMenu } from './panel-menu.js';
import '../ui/select.js';

export function button(label, action, signal) {
  const element = document.createElement('button');
  element.type = 'button';
  element.textContent = label;
  element.addEventListener('click', action, { signal });
  return element;
}

export function createPanel(record, { services, storage, changed, onOpen, surfaceOnly = false, createOwned, releaseOwned }) {
  const controller = new AbortController();
  const { signal } = controller;
  const element = document.createElement('section');
  element.className = 'extension-panel';
  element.dataset.extensionId = record.id;
  element.dataset.presentation = record.manifest?.presentation ?? 'widget';
  const header = document.createElement('div');
  header.className = 'extension-panel-header';
  const title = button(record.manifest?.name ?? record.id, () => {
    changed({ collapsed: !record.collapsed });
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
  compactStyle.textContent = ':host{display:grid;place-items:center;color:inherit;font:inherit}svg{width:28px;height:28px}.extension-host-icon{width:20px;height:20px}';
  const compactIcon = icon('panel'); compactIcon.classList.add('extension-host-icon');
  compact.append(compactStyle, compactIcon);
  const placementMenu = attachPanelMenu(header, record, changed, signal);
  for (const control of [title, compactButton]) {
    control.setAttribute('aria-description', 'ПКМ или Shift+F10 — расположение панели');
  }
  header.append(compactButton, title);
  const reveal = document.createElement('div'); reveal.className = 'extension-panel-reveal';
  const inner = document.createElement('div'); inner.className = 'extension-panel-inner';
  inner.append(body, error, retry); reveal.append(inner);
  element.append(header, reveal);
  let runtime, panelRoot;
  let failed = false;

  function mount() {
    releaseOwned?.();
    runtime?.stop();
    // Новый root для каждого mount: поздняя очистка отменённой async-панели
    // может затронуть только отсоединённое дерево, а не следующую инстанцию.
    const surface = document.createElement('div');
    surface.className = 'extension-panel-content';
    body.replaceChildren(surface);
    const shadow = surface.attachShadow({ mode: 'open' });
    panelRoot = shadow;
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
    if (surfaceOnly) return;
    runtime = createPanelRuntime({
      panels: Object.freeze({ create: createOwned }),
      manifest: record.manifest, root: shadow, compact,
      panel: Object.freeze({ open: () => compactButton.click(), move: location => changed({ location, collapsed: false }) }), services,
      storage: extensionStorage(storage, record.id),
      onError(failure) {
        releaseOwned?.();
        shadow.replaceChildren();
        error.textContent = `Не удалось открыть панель: ${failure.message}`;
        failed = true;
        retry.hidden = record.collapsed;
      },
    });
  }

  function update() {
    placementMenu.close();
    title.setAttribute('aria-expanded', String(!record.collapsed));
    title.title = record.collapsed ? 'Развернуть панель' : 'Свернуть панель';
    toggle.replaceChildren(icon(record.collapsed ? 'chevron-right' : 'chevron-down'));
    if (record.collapsed && inner.contains(document.activeElement)) title.focus({ preventScroll: true });
    inner.inert = record.collapsed;
    inner.setAttribute('aria-hidden', String(record.collapsed));
    element.classList.toggle('expanded', !record.collapsed);
    error.hidden = record.collapsed;
    retry.hidden = record.collapsed || !failed;
  }
  // The compact surface needs the same live instance even when initially collapsed.
  mount();
  update();
  return { element, root: panelRoot, update, stop() { controller.abort(); releaseOwned?.(); runtime?.stop(); element.remove(); } };
}
