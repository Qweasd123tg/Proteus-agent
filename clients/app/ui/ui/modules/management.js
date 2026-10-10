import { hasSurface } from '../../extensions/contract.js';

// A choice is shown only when another installed module provides the same surface.
export function selectionButtons(root, record, registry, signal) {
  const records = registry.state().records ?? [];
  for (const slot of ['composer-model', 'composer-access']) {
    if (!hasSurface(record.manifest, slot)) continue;
    if (records.filter(item => hasSurface(item.manifest, slot)).length < 2) continue;
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
