import { mountExtensions } from './host.js';
import { createExtensionRegistry } from './registry.js';
import { mountExtensionSettings } from './settings.js';
import { mountReport } from './usage/report.js';
import { extensionStorage } from './storage.js';
import { theme } from './theme.js';

import { sessionStateService } from './session-state.js';
export { publishSessionState } from './session-state.js';

const registry = createExtensionRegistry();
export function mountWebExtensionSettings(root) { return mountExtensionSettings(root, registry); }

// Адаптер этой витрины. Credentials остаются в transport-коде клиента;
// расширение получает только объявленный интерфейс чтения публичного API.
export function mountWebExtensions(root, readConfig, readQuota, readUsage, readWorkspace) {
  const reader = callback => signal => Object.freeze({
    async read() {
      signal.throwIfAborted();
      const value = await callback(signal);
      signal.throwIfAborted();
      return JSON.parse(value);
    },
  });
  const services = {
    'agent.config.read': reader(readConfig),
    'agent.model.quota.read': reader(readQuota),
    'agent.usage.read': reader(readUsage),
    'agent.session.read': sessionStateService,
    'agent.workspace.read': signal => Object.freeze({
      list: path => readWorkspace('/workspace/list?path=' + encodeURIComponent(path), signal).then(JSON.parse),
      read: path => readWorkspace('/workspace/file?path=' + encodeURIComponent(path), signal).then(JSON.parse),
      changes: () => readWorkspace('/workspace/changes', signal).then(JSON.parse),
      diff: path => readWorkspace('/workspace/diff?path=' + encodeURIComponent(path), signal).then(JSON.parse),
    }),
  };
  const locations = Object.fromEntries(['left','right'].map(side => [side, document.querySelector(`[data-extension-location=${side}]`)]));
  const columns = Object.fromEntries(['left','right'].map(side => [side, document.querySelector(`[data-extension-columns=${side}]`)]));
  const stop = mountExtensions(root, services, { registry, locations, columns, onOpen(location) {
    const dock = document.querySelector(location === 'right' ? '.info-panel:not(.open)' : location === 'left' ? '.app-layout.sidebar-collapsed .sidebar' : ':not(*)');
    [...(dock?.querySelectorAll('[data-panel-toggle]') ?? [])].find(button => !button.closest('[inert]'))?.click();
    if (location === 'left') document.querySelector('.sidebar-view-tabs button:last-child')?.click();
  } });
  return stop;
}

export function mountUsageDetails(root, readUsage) {
  const controller = new AbortController();
  const surface = document.createElement('div'); root.replaceChildren(surface);
  const shadow = surface.attachShadow({ mode: 'open' });
  const style = document.createElement('style'); style.textContent = theme; shadow.append(style);
  const { signal } = controller;
  const stop = mountReport({ root: shadow, signal, storage: extensionStorage(localStorage, 'usage'), services: {
    'agent.usage.read': { async read() { signal.throwIfAborted(); const value = await readUsage(signal); signal.throwIfAborted(); return JSON.parse(value); } },
  } }, true);
  return () => { controller.abort(); stop(); root.replaceChildren(); };
}
