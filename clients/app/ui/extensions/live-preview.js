import { viewForSurface } from './contract.js';
import { createViewRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { createViewRoot } from './view-root.js';
import { icon } from './icons.js';
import { createDemoServices, demoServiceNames } from './demo-services.js';

const SINGLE = ['settings', 'composer-model', 'composer-access'];

// What the extension shows when enabled: its tab with the widget, otherwise
// its first other surface. Combined compact/workspace views mount once.
function previewViews(manifest) {
  const workspace = viewForSurface(manifest, 'workspace'), compact = viewForSurface(manifest, 'compact');
  if (workspace || compact) return { workspace, compact, views: [...new Set([workspace, compact].filter(Boolean))] };
  const surface = SINGLE.find(name => viewForSurface(manifest, name));
  return { surface, views: surface ? [viewForSurface(manifest, surface)] : [] };
}

// Builtins use host state directly, so only packages, and only when every
// declared interface has a demo implementation, can be tried.
export function canPreview(record) {
  const { views } = previewViews(record.manifest);
  return record.source === 'package' && !!views.length && views.every(view => view.requires.every(name => demoServiceNames.has(name)));
}

function memoryStorage() {
  const values = new Map();
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, String(value)), removeItem: key => values.delete(key) };
}

export function mountLivePreview(container, record, onClose) {
  const controller = new AbortController(), { signal } = controller;
  const services = createDemoServices(signal), storage = memoryStorage();
  const { workspace, compact, surface, views } = previewViews(record.manifest);
  const name = record.manifest.name;
  const frame = document.createElement('section');
  frame.className = 'extension-demo'; frame.dataset.extensionDemo = record.id;
  frame.setAttribute('aria-label', `Демо: ${name}`);
  const bar = document.createElement('div'); bar.className = 'extension-demo-bar';
  const badge = document.createElement('span'); badge.className = 'extension-demo-badge'; badge.textContent = 'Демо';
  const note = document.createElement('span'); note.className = 'extension-demo-note'; note.textContent = 'Вымышленные данные, ничего не сохраняется';
  const close = document.createElement('button'); close.type = 'button'; close.className = 'extension-demo-close'; close.textContent = 'Закрыть';
  close.addEventListener('click', onClose, { signal });
  const body = document.createElement('div'); body.className = 'extension-demo-body';
  body.dataset.viewLayout = views[0]?.layout ?? 'scroll';
  const error = document.createElement('p'); error.className = 'settings-status'; error.setAttribute('role', 'alert'); error.hidden = true;
  let widget, compactRoot, hover;
  if (compact) {
    widget = document.createElement('button'); widget.type = 'button'; widget.className = 'extension-widget';
    widget.title = name; widget.setAttribute('aria-label', name);
    const holder = document.createElement('span'); widget.append(holder);
    compactRoot = holder.attachShadow({ mode: 'open' });
    hover = Object.freeze({ set(text) { if (typeof text !== 'string') throw new Error('Подсказка должна быть текстом'); widget.dataset.uiTooltipDetails = text; } });
    hover.set(record.manifest.description ?? '');
    widget.addEventListener('click', () => body.scrollIntoView({ block: 'nearest' }), { signal });
    const label = document.createElement('span'); label.className = 'extension-demo-widget'; label.append('Виджет', widget);
    bar.append(badge, note, label, close);
  } else bar.append(badge, note, close);
  frame.append(bar, body, error); container.append(frame);
  // Tabs, panel moves and writes have no effect outside the demo.
  const panels = Object.freeze({ create() { throw new Error('Отдельные вкладки недоступны в демо'); } });
  const panel = Object.freeze({ open() {}, move() {} });
  let runtimes = [];
  const stopViews = () => { for (const runtime of runtimes) runtime.stop(); runtimes = []; };
  function start() {
    stopViews(); body.replaceChildren(); error.hidden = true;
    if (compactRoot) compactRoot.replaceChildren(icon(record.manifest.icon || 'modules'));
    if (compact && !workspace) {
      const hint = document.createElement('p'); hint.className = 'settings-hint';
      hint.textContent = 'У расширения только виджет: наведите на иконку выше, чтобы увидеть подробности.';
      body.append(hint);
    }
    for (const view of views) {
      const content = createViewRoot(body, view.isolation, 'extension-panel-content');
      if (view !== workspace && (workspace || compact)) content.element.hidden = true;
      runtimes.push(createViewRuntime({
        view, root: content.root, services, panels,
        surface: surface ?? (view === workspace ? 'workspace' : 'compact'),
        compact: view === compact ? compactRoot : undefined, hover: view === compact ? hover : undefined,
        panel: view === workspace ? panel : undefined,
        storage: extensionStorage(storage, record.id),
        onError(failure) {
          if (signal.aborted) return;
          stopViews(); body.replaceChildren();
          error.textContent = `Не удалось запустить демо: ${failure.message}`; error.hidden = false;
          const retry = document.createElement('button'); retry.type = 'button'; retry.textContent = 'Повторить';
          retry.addEventListener('click', start, { signal }); body.append(retry);
        },
      }));
    }
  }
  start();
  close.focus({ preventScroll: true });
  return () => { controller.abort(); stopViews(); frame.remove(); };
}
