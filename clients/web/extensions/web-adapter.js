import { mountExtensions } from './host.js';
import { createExtensionRegistry } from './registry.js';
import { mountExtensionSettings } from './settings.js';

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
  const stop = mountExtensions(root, services, { registry, target: document.querySelector('[data-extension-columns=right]') });
  return stop;
}
