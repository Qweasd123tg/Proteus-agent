const SETTINGS_KEY = 'proteus.ui.extensions';

export function settingsStore(storage) {
  return {
    read() { return storage.getItem(SETTINGS_KEY); },
    write(panels) { storage.setItem(SETTINGS_KEY, JSON.stringify({ apiVersion: 1, panels })); },
  };
}

// Данные расширений принадлежат клиенту и не попадают в config/history агента.
export function extensionStorage(storage, id) {
  const prefix = `proteus.ui.extension.${id}:`;
  return Object.freeze({
    get(key) { return storage.getItem(prefix + key); },
    set(key, value) { storage.setItem(prefix + key, String(value)); },
    remove(key) { storage.removeItem(prefix + key); },
  });
}
