import { mountExtensions } from './host.js';
import { createExtensionRegistry } from './registry.js';
import { mountExtensionSettings } from './settings.js';

const registry = createExtensionRegistry();
export function mountWebExtensionSettings(root) { return mountExtensionSettings(root, registry); }

// Адаптер этой витрины. Credentials остаются в transport-коде клиента;
// расширение получает только объявленный интерфейс чтения публичного API.
export function mountWebExtensions(root, readConfig, readQuota) {
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
  };
  return mountExtensions(root, services, { registry });
}
