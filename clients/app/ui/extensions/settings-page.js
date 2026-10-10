import { hasSurface } from './contract.js';
import { widgetPlacement } from './widgets.js';
import { mountView } from './view-host.js';
import { logicallyVisible } from '../ui/visibility.js';
import { mountExtensionDetails } from './details.js';

// The host information stays mounted while enabled views retain their own lifecycle.
export function mountExtensionOptions(root, record, registry, services = {}) {
  const controller = new AbortController(), { signal } = controller;
  root.classList.add('extension-settings-page');
  const stopDetails = mountExtensionDetails(root, record, registry);
  const body = document.createElement('div'); body.className = 'extension-settings-body';
  if (record.source === 'builtin') body.classList.add('builtin-settings-body');
  root.append(body);
  let enabled, stopView, bodyController;
  const unsubscribe = registry.subscribe(() => {
    const next = !!registry.state().records.find(item => item.id === record.id)?.enabled;
    if (next === enabled) return;
    enabled = next; bodyController?.abort(); bodyController = new AbortController();
    stopView?.(); stopView = undefined; body.replaceChildren();
    if (enabled && hasSurface(record.manifest, 'compact')) body.append(widgetPlacement(registry.storage, bodyController.signal, record.id, record.widget));
    if (enabled && hasSurface(record.manifest, 'settings')) stopView = mountView(body, record, registry.storage, services, 'settings');
    else if (!enabled || !hasSurface(record.manifest, 'compact')) {
      // Composer pickers are configured in the chat input; their description already says so.
      const composer = hasSurface(record.manifest, 'composer-model') || hasSurface(record.manifest, 'composer-access');
      if (enabled && composer) return;
      const hint = document.createElement('p'); hint.className = 'settings-hint';
      hint.textContent = !enabled ? 'Расширение выключено. Включите его, чтобы открыть параметры.'
        : 'У этого расширения нет дополнительных параметров.';
      body.append(hint);
    }
  });
  document.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented || !logicallyVisible(root)) return;
    event.preventDefault(); event.stopPropagation();
    document.dispatchEvent(new CustomEvent('proteus-select-settings-module', { detail: 'extensions' }));
    root.closest('.settings-page')?.querySelector('[data-settings-section=extensions]')?.focus();
  }, { signal });
  return () => { controller.abort(); unsubscribe(); stopView?.(); bodyController?.abort(); stopDetails(); root.replaceChildren(); root.classList.remove('extension-settings-page'); };
}
