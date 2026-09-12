import { createExtensionRegistry } from './registry.js';
import { createPanel } from './panel.js';
import { createColumn } from './column.js';

export function mountExtensions(root, services = {}, options = {}) {
  const registry = options.registry ?? createExtensionRegistry(options);
  const cards = new Map(), columns = new Map(), owned = new Map();
  const notice = document.createElement('p'); notice.className = 'extension-surface-status'; notice.setAttribute('role', 'status');
  const panels = document.createElement('div'); panels.className = 'extension-panels'; root.append(notice, panels);
  let stopped = false;
  const allRecords = () => [...registry.state().records.filter(record => record.enabled), ...[...owned.values()].map(item => item.record)];
  function update(id, change) {
    const record = allRecords().find(record => record.id === id);
    if (!record) return;
    if (owned.has(id)) Object.assign(record, change);
    else registry.update(id, change);
    render();
    if (change.collapsed === false) columns.get(id)?.reveal();
  }
  function release(owner) {
    let changed = false;
    for (const [id, item] of owned) if (item.owner === owner) { item.card.stop(); columns.get(id)?.stop(); columns.delete(id); owned.delete(id); changed = true; }
    if (changed && !stopped) queueMicrotask(render);
  }
  function createOwned(owner, key, { title, location = 'right' }) {
    if (stopped || !cards.has(owner.id)) throw new Error('Расширение закрыто');
    if (!/^[a-z0-9][a-z0-9.-]*$/.test(key) || typeof title !== 'string' || !title.trim() || !['left','right'].includes(location)) throw new Error('Некорректная панель');
    const id = `${owner.id}:${key}`;
    if (owned.has(id)) return owned.get(id).handle;
    const record = { id, location, enabled: true, collapsed: true, manifest: { name: title, presentation: 'panel' } };
    const card = createPanel(record, { surfaceOnly: true, changed: change => update(id, change) });
    const handle = Object.freeze({ root: card.root,
      show() { if (owned.get(id)?.record !== record) return; update(id, { collapsed: false }); if (record.manifest?.presentation !== 'panel') options.onOpen?.(record.location); },
      hide() { if (owned.get(id)?.record === record) update(id, { collapsed: true }); },
    });
    owned.set(id, { owner, record, card, handle }); render(); return handle;
  }
  function render() {
    if (stopped) return;
    const state = registry.state(); notice.textContent = state.notice || (!state.ready ? 'Загрузка панелей…' : '');
    for (const [id, card] of cards) if (!state.records.some(record => record.id === id && record.enabled && record === card.record)) {
      release(card.record); card.stop(); columns.get(id)?.stop(); columns.delete(id); cards.delete(id);
    }
    for (const record of state.records.filter(record => record.enabled)) if (!cards.has(record.id)) {
      const card = createPanel(record, { services, storage: registry.storage,
        changed: change => update(record.id, change), onOpen: location => { if (record.manifest?.presentation !== 'panel') options.onOpen?.(location); },
        createOwned: (key, spec) => createOwned(record, key, spec), releaseOwned: () => release(record) });
      cards.set(record.id, { ...card, record });
    }
    const positions = new Map(), records = allRecords();
    for (const [id, column] of columns) if (!records.some(record => record.id === id)) { column.stop(); columns.delete(id); }
    for (const record of records) {
      const card = cards.get(record.id) ?? owned.get(record.id).card; card.update();
      const independent = record.manifest?.presentation === 'panel';
      let element = card.element;
      if (independent) {
        if (!columns.has(record.id)) columns.set(record.id, createColumn(record, card, registry.storage));
        const column = columns.get(record.id); column.update(); element = column.element;
      }
      const target = (independent ? options.columns?.[record.location] : options.locations?.[record.location]) ?? panels;
      card.element.dataset.location = record.location;
      const index = positions.get(target) ?? 0; positions.set(target, index + 1);
      if (target.children[index] !== element) target.insertBefore(element, target.children[index] ?? null);
    }
  }
  const unsubscribe = registry.subscribe(render);
  void registry.start();
  return () => {
    stopped = true; unsubscribe();
    for (const card of cards.values()) { release(card.record); card.stop(); }
    for (const column of columns.values()) column.stop();
    if (!options.registry) registry.dispose(); root.replaceChildren();
  };
}
