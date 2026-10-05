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
