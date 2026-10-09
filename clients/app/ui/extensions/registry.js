import { parseManifest, parseSettings } from './contract.js';
import { settingsStore } from './storage.js';
import { nativeArchivePackages } from './archive-packages.js';

async function readJson(url, signal) {
  const response = await fetch(url, { signal, credentials: 'omit', cache: 'no-store' });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return { value: await response.json(), url };
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
  const packages = options.packages ?? nativeArchivePackages();
  const store = settingsStore(storage);
  const listeners = new Set();
  let records = [], bundled = [], notice = '', busy = false, ready = false, started;
  const emit = () => { if (!controller.signal.aborted) for (const listener of listeners) listener(); };
  function save() {
    try {
      store.write(serialized(records));
      notice = '';
    } catch { notice = 'Изменения действуют до закрытия: не удалось сохранить настройки.'; }
    emit();
  }
  const serialized = records => records.map(({ id, url, enabled, collapsed, location, packageKey }) => ({ id, url, enabled, collapsed, location, ...(packageKey ? { packageKey } : {}) }));
  async function resolve(record) {
    try {
      const response = await read(record.url, controller.signal);
      const manifest = parseManifest(response.value, response.url);
      if (options.reservedIds?.includes(manifest.id)) throw new Error('Идентификатор занят встроенным модулем');
      if (manifest.id !== record.id) throw new Error(`id манифеста изменился: ${manifest.id}`);
      return { ...record, source: 'package', manifest };
    } catch (error) { return { ...record, source: 'package', error: `Манифест: ${error.message}` }; }
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
      if (next.some(record => options.reservedIds?.includes(record.id))) throw new Error('Список содержит идентификатор встроенного модуля');
      const removedKeys = defaults ? records.filter(r => r.packageKey).map(r => r.packageKey) : [];
      if (defaults) store.write(serialized(next));
      records = next; ready = true; notice = catalogError;
      if (removedKeys.length) {
        emit(); // Replacing the list stops runtimes before their files go away.
        for (const key of removedKeys) {
          try { await packages.remove(key); }
          catch (error) { notice += ` Файлы ZIP-пакета не удалены: ${error.message ?? error}`; }
        }
      }
    } catch (error) { notice = `Не удалось загрузить расширения: ${error.message}`; }
    finally { busy = false; emit(); }
  }
  return {
    storage,
    state: () => ({ records, bundled, notice, busy, ready, archiveAvailable: packages.available }),
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
      const [record] = records.splice(index, 1); records.splice(next, 0, record); save();
    },
    async remove(id) {
      if (busy) return;
      const record = records.find(item => item.id === id);
      if (!record) return;
      const next = records.filter(item => item !== record);
      try { store.write(serialized(next)); }
      catch { notice = 'Не удалось сохранить удаление. Расширение осталось в списке.'; emit(); return; }
      records = next; notice = ''; emit(); // Abort every view before removing files.
      if (record.packageKey) {
        busy = true; emit();
        try { await packages.remove(record.packageKey); }
        catch (error) { notice = `Пакет убран из списка, но его файлы не удалены: ${error.message ?? error}`; }
        finally { busy = false; emit(); }
      }
    },
    addBundled(id) {
      if (!ready || busy || records.some(item => item.id === id)) return;
      const record = bundled.find(item => item.id === id);
      if (!record || record.error) return;
      records.push({ ...record, enabled: true, collapsed: false }); save();
    },
    async install(file) {
      if (busy || !ready) return false;
      busy = true; notice = ''; emit();
      let installed, accepted = false;
      try {
        installed = await packages.install(file, [...(options.reservedIds ?? []), ...records.map(r => r.id)], controller.signal);
        const { url, id, key } = installed;
        const response = await read(url, controller.signal);
        const manifest = parseManifest(response.value, response.url);
        controller.signal.throwIfAborted();
        if (manifest.id !== id) throw new Error('id установленного пакета не совпал с манифестом');
        if (options.reservedIds?.includes(manifest.id)) throw new Error('Это имя занято встроенным модулем');
        if (records.some(record => record.id === manifest.id)) throw new Error(`Расширение ${manifest.id} уже добавлено`);
        const next = [...records, { id, source: 'package', packageKey: key, url, manifest, enabled: true, collapsed: false, location: 'right' }];
        store.write(serialized(next));
        records = next; accepted = true; return true;
      } catch (error) { notice = `Не удалось установить ZIP: ${error.message ?? error}`; return false; }
      finally {
        if (installed && !accepted) {
          try { await packages.remove(installed.key); }
          catch (error) { notice += ` Файлы не удалены: ${error.message ?? error}`; }
        }
        busy = false; emit();
      }
    },
    reset: () => initialize(true),
    dispose() { controller.abort(); listeners.clear(); },
  };
}
