const SETTINGS_KEY = 'proteus.ui.extensions';
const subscribers = new WeakMap();
const externalListeners = new WeakMap();

export function settingsStore(storage) {
  return {
    read() { return storage.getItem(SETTINGS_KEY); },
    write(panels) { storage.setItem(SETTINGS_KEY, JSON.stringify({ apiVersion: 1, panels })); },
  };
}

// Данные расширений принадлежат клиенту и не попадают в config/history агента.
export function extensionStorage(storage, id) {
  const prefix = `proteus.ui.extension.${id}:`;
  if (!subscribers.has(storage)) subscribers.set(storage, new Map());
  const listenersById = subscribers.get(storage);
  const changed = () => { for (const callback of listenersById.get(id) ?? []) callback(); };
  return Object.freeze({
    get(key) { return storage.getItem(prefix + key); },
    set(key, value) { storage.setItem(prefix + key, String(value)); changed(); },
    remove(key) { storage.removeItem(prefix + key); changed(); },
    subscribe(callback) {
      if (!listenersById.has(id)) listenersById.set(id, new Set());
      listenersById.get(id).add(callback);
      if(!externalListeners.has(storage)&&globalThis.addEventListener){
        const listener=event=>{
          if(event.storageArea!==storage)return;
          for(const [namespace,callbacks]of listenersById){
            if(event.key!==null&&!event.key?.startsWith(`proteus.ui.extension.${namespace}:`))continue;
            for(const callback of callbacks)callback();
          }
        };
        globalThis.addEventListener('storage',listener);
        externalListeners.set(storage,listener);
      }
      return () => {
        const listeners = listenersById.get(id); listeners?.delete(callback);
        if (!listeners?.size) listenersById.delete(id);
        if(!listenersById.size){
          const listener=externalListeners.get(storage);
          if(listener)globalThis.removeEventListener('storage',listener);
          externalListeners.delete(storage);
        }
      };
    },
  });
}
