import { hasSurface } from './contract.js';
import { widgetPlacement } from './widgets.js';
import { mountView } from './view-host.js';
import { logicallyVisible } from '../ui/modules/visibility.js';

// Each enabled package owns an ordinary retained settings section.
export function mountExtensionOptions(root, record, storage, services = {}) {
  const controller = new AbortController(), { signal } = controller;
  root.classList.add('extension-settings-page');
  if (hasSurface(record.manifest, 'compact')) root.append(widgetPlacement(storage, signal, record.id));
  const stop = hasSurface(record.manifest, 'settings') ? mountView(root, record, storage, services, 'settings') : undefined;
  if (!stop && !hasSurface(record.manifest, 'compact')) {
    const hint = document.createElement('p');
    hint.className = 'settings-hint';
    hint.textContent = 'У этого расширения нет дополнительных параметров.';
    root.append(hint);
  }
  document.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented || !logicallyVisible(root)) return;
    event.preventDefault(); event.stopPropagation();
    document.dispatchEvent(new CustomEvent('proteus-select-settings-module', { detail: 'extensions' }));
    root.closest('.settings-page')?.querySelector('[data-settings-section=extensions]')?.focus();
  }, { signal });
  return () => { controller.abort(); stop?.(); root.replaceChildren(); root.classList.remove('extension-settings-page'); };
}
