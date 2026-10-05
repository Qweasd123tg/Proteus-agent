import { hasSurface } from '../../extensions/contract.js';

export function selectionButtons(root, record, registry, signal) {
  for (const slot of ['composer-model', 'composer-access']) {
    if (!hasSurface(record.manifest, slot)) continue;
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.selectSlot = slot;
    button.dataset.moduleId = record.id;
    const selected = registry.state().slots?.[slot] === record.id;
    button.textContent = selected ? 'Выбран' : 'Использовать';
    button.disabled = !record.enabled || selected;
    button.setAttribute('aria-pressed', String(selected));
    button.addEventListener('click', () => registry.select(slot, record.id), { signal });
    root.append(button);
  }
}

export function mountBuiltinControl(root, record, registry) {
  const controller = new AbortController();
  let choicesController;
  const row = document.createElement('div');
  row.className = 'settings-row'; row.dataset.builtinModule = record.id;
  const label = document.createElement('label'); label.className = 'settings-label';
  const name = document.createElement('strong'); name.textContent = record.manifest.name;
  const hint = document.createElement('span'); hint.className = 'settings-hint'; hint.textContent = record.manifest.description;
  label.append(name, hint);
  const input = document.createElement('input');
  input.type = 'checkbox'; input.className = 'settings-toggle'; input.id = 'module-' + record.id;
  input.setAttribute('aria-label', 'Включить: ' + record.manifest.name); label.htmlFor = input.id;
  input.disabled = record.required;
  input.addEventListener('change', () => registry.update(record.id, { enabled: input.checked }), { signal: controller.signal });
  const choices = document.createElement('span'); choices.className = 'module-selection';
  row.append(label, choices, input); root.append(row);
  const unsubscribe = registry.subscribe(() => {
    choicesController?.abort(); choicesController = new AbortController();
    input.checked = record.enabled;
    choices.replaceChildren(); selectionButtons(choices, record, registry, choicesController.signal);
  });
  return () => { controller.abort(); choicesController?.abort(); unsubscribe(); root.replaceChildren(); };
}
