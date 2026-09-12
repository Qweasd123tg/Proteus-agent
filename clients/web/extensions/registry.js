import { parseManifest, parseSettings, resourceUrl } from './contract.js';
import { settingsStore } from './storage.js';

async function readJson(url, signal) {
  const response = await fetch(url, { signal, credentials: 'omit', cache: 'no-store' });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return { value: await response.json(), url: response.url };
}

// Client-owned data/lifecycle, shared by the panel surface and settings.
// Loading settings never executes extension entry points.
export function createExtensionRegistry(options = {}) {
  const controller = new AbortController();
  const catalogUrl = options.catalogUrl ?? new URL('./catalog.json', import.meta.url).href;
  const storage = options.storage ?? {
    getItem: key => localStorage.getItem(key), setItem: (key, value) => localStorage.setItem(key, value),
    removeItem: key => localStorage.removeItem(key),
  };
  const read = options.readJson ?? readJson;
  const store = settingsStore(storage);
  const listeners = new Set();
  let records = [], bundled = [], notice = '', busy = false, ready = false, started;
  const emit = () => { if (!controller.signal.aborted) for (const listener of listeners) listener(); };
  function save() {
    try {
      store.write(records.map(({ id, url, enabled, collapsed, location }) => ({ id, url, enabled, collapsed, location })));
      notice = '';
    } catch { notice = 'Изменения действуют до закрытия: не удалось сохранить настройки.'; }
    emit();
  }
  async function resolve(record) {
    try {
      const response = await read(record.url, controller.signal);
      const manifest = parseManifest(response.value, response.url);
      if (manifest.id !== record.id) throw new Error(`id манифеста изменился: ${manifest.id}`);
      return { ...record, manifest };
    } catch (error) { return { ...record, error: `Манифест: ${error.message}` }; }
  }
  async function initialize(defaults = false) {
    if (busy) return;
    busy = true; emit();
    try {
      const saved = defaults ? null : store.read();
      let catalogError = '';
      try {
        const catalog = (await read(catalogUrl, controller.signal)).value;
        bundled = await Promise.all(parseSettings(catalog, catalogUrl).map(resolve));
      } catch (error) {
        if (saved === null) throw error;
        bundled = [];
        catalogError = `Список поставляемых расширений недоступен: ${error.message}`;
      }
      const next = saved === null ? bundled.map(record => ({ ...record }))
        : await Promise.all(parseSettings(JSON.parse(saved), catalogUrl).map(record =>
          bundled.find(item => item.id === record.id && item.url === record.url)?.manifest
            ? { ...bundled.find(item => item.id === record.id && item.url === record.url), ...record }
            : resolve(record)));
      if (controller.signal.aborted) return;
      records = next; ready = true; notice = catalogError;
      if (defaults) save();
    } catch (error) { notice = `Не удалось загрузить расширения: ${error.message}`; }
    finally { busy = false; emit(); }
  }
  return {
    storage,
    state: () => ({ records, bundled, notice, busy, ready }),
    start() { return started ??= initialize(); },
    subscribe(listener) { listeners.add(listener); listener(); return () => listeners.delete(listener); },
    update(id, change) {
      if (!ready || busy) return;
      const record = records.find(item => item.id === id);
      if (!record) return;
      if (typeof change.enabled === 'boolean') record.enabled = change.enabled;
      if (typeof change.collapsed === 'boolean') record.collapsed = change.collapsed;
      if (['left', 'right'].includes(change.location)) record.location = change.location;
      save();
    },
    move(id, step) {
      if (busy) return;
      const index = records.findIndex(item => item.id === id), next = index + step;
      if (index < 0 || next < 0 || next >= records.length) return;
      [records[index], records[next]] = [records[next], records[index]]; save();
    },
    remove(id) { if (!busy) { records = records.filter(item => item.id !== id); save(); } },
    addBundled(id) {
      if (!ready || busy || records.some(item => item.id === id)) return;
      const record = bundled.find(item => item.id === id);
      if (!record || record.error) return;
      records.push({ ...record, enabled: true, collapsed: false }); save();
    },
    async install(value) {
      if (busy || !ready) return false;
      busy = true; notice = ''; emit();
      try {
        const url = resourceUrl(value, catalogUrl);
        const response = await read(url, controller.signal);
        const manifest = parseManifest(response.value, response.url);
        if (controller.signal.aborted) return false;
        if (records.some(record => record.id === manifest.id)) throw new Error(`Расширение ${manifest.id} уже добавлено`);
        records.push({ id: manifest.id, url, manifest, enabled: true, collapsed: false, location: 'right' });
        save(); return true;
      } catch (error) { notice = `Не удалось добавить расширение: ${error.message}`; return false; }
      finally { busy = false; emit(); }
    },
    reset: () => initialize(true),
    dispose() { controller.abort(); listeners.clear(); },
  };
}
