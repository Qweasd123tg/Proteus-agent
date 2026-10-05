import { hasSurface } from '../../extensions/contract.js';
import { mountBuiltinControl } from './management.js';
import { mountModule } from './host.js';

export function builtinSettingsSurface(record) {
  return ['settings', 'composer-model', 'composer-access'].find(surface => hasSurface(record.manifest, surface));
}

// Host controls remain available while an optional built-in module is off.
export function mountBuiltinSettings(root, record, registry, services) {
  root.classList.add('builtin-settings-page');
  const controls = document.createElement('div'), body = document.createElement('div');
  controls.className = 'builtin-settings-controls';
  body.className = 'builtin-settings-body';
  if (!record.required) root.append(controls);
  root.append(body);
  const stopControls = record.required ? () => {} : mountBuiltinControl(controls, record, registry);
  let enabled, stopModule;
  const unsubscribe = registry.subscribe(() => {
    const next = !!registry.state().records.find(item => item.id === record.id)?.enabled;
    if (next === enabled) return;
    enabled = next;
    stopModule?.(); stopModule = undefined; body.replaceChildren();
    if (enabled && hasSurface(record.manifest, 'settings')) stopModule = mountModule(body, record, registry, services, 'settings');
    else {
      const hint = document.createElement('p');
      hint.className = 'settings-hint';
      hint.textContent = enabled ? 'Выбор модели и прав доступен в поле ввода чата.' : 'Встроенное расширение выключено.';
      body.append(hint);
    }
  });
  return () => { unsubscribe(); stopModule?.(); stopControls(); root.replaceChildren(); };
}
