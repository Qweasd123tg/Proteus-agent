import { mountExtensions } from './host.js';
import { createExtensionRegistry } from './registry.js';
import { mountExtensionSettings } from './settings.js';
import { mountReport } from './usage/report.js';
import { extensionStorage } from './storage.js';
import { theme } from './theme.js';

const registry = createExtensionRegistry();
export function mountWebExtensionSettings(root) { return mountExtensionSettings(root, registry); }

// Адаптер этой витрины. Credentials остаются в transport-коде клиента;
// расширение получает только объявленный интерфейс чтения публичного API.
export function mountWebExtensions(root, readConfig, readQuota, readUsage) {
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
  };
  return mountExtensions(root, services, { registry });
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
