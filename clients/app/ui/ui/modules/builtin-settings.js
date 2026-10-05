import { hasSurface } from '../../extensions/contract.js';
import { mountView } from '../../extensions/view-host.js';
import { mountExtensionOptions } from '../../extensions/settings-page.js';

export function builtinSettingsSurface(record) {
  return ['settings', 'composer-model', 'composer-access'].find(surface => hasSurface(record.manifest, surface));
}

// Agent profile editors are permanent pages; client extensions share the package page.
export function mountBuiltinSettings(root, record, registry, services) {
  return record.settingsGroup === 'agent'
    ? mountView(root, record, registry.storage, services, 'settings')
    : mountExtensionOptions(root, record, registry, services);
}
