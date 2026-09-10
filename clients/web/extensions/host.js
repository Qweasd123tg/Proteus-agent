import { createExtensionRegistry } from './registry.js';
import { createPanel } from './panel.js';

// Display surface only. Installation and arrangement belong to Settings.
export function mountExtensions(root, services = {}, options = {}) {
  const registry = options.registry ?? createExtensionRegistry(options);
  const cards = new Map();
  const notice = document.createElement('p');
  notice.className = 'extension-surface-status';
  notice.setAttribute('role', 'status');
  const panels = document.createElement('div');
  panels.className = 'extension-panels';
  root.append(notice, panels);
  const unsubscribe = registry.subscribe(() => {
    const state = registry.state();
    notice.textContent = state.notice || (!state.ready ? 'Загрузка панелей…' : '');
    for (const [id, card] of cards) {
      if (!state.records.some(record => record.id === id && record.enabled && record === card.record)) {
        card.stop(); cards.delete(id);
      }
    }
    for (const record of state.records.filter(record => record.enabled)) {
      if (!cards.has(record.id)) {
        const card = createPanel(record, { services, storage: registry.storage,
          changed: () => registry.update(record.id, { collapsed: record.collapsed }) });
        cards.set(record.id, { ...card, record });
      }
      const card = cards.get(record.id);
      card.update(); panels.append(card.element);
    }
  });
  void registry.start();
  return () => {
    unsubscribe();
    for (const card of cards.values()) card.stop();
    if (!options.registry) registry.dispose();
    root.replaceChildren();
  };
}
