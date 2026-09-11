import { missingServices } from './contract.js';

// Никаких импортов Leptos, Tauri или agent runtime. Другой клиент может
// предоставить те же интерфейсы своим transport-адаптером.
export function createPanelRuntime({ manifest, root, compact, panel, services, storage, load = url => import(url), onError }) {
  const controller = new AbortController();
  const { signal } = controller;
  let cleanup;
  let stopped = false;
  function dispose() {
    if (!cleanup) return;
    const callback = cleanup;
    cleanup = undefined;
    try { callback(); } catch (error) { if (!stopped) onError(error); }
  }
  const ready = (async () => {
    try {
      const missing = missingServices(manifest, services);
      if (missing.length) throw new Error(`Клиент не предоставляет: ${missing.join(', ')}`);
      const implementation = await load(manifest.entry);
      if (stopped) return;
      if (typeof implementation.mount !== 'function') throw new Error('Расширение не экспортирует mount');
      const selected = Object.fromEntries(manifest.requires.map(name => [name, services[name](signal)]));
      const result = await implementation.mount(Object.freeze({ root, compact, panel, services: Object.freeze(selected), storage, signal }));
      if (result !== undefined && typeof result !== 'function') throw new Error('mount должен вернуть функцию очистки или undefined');
      cleanup = result;
      if (stopped) dispose();
    } catch (error) {
      if (!stopped) {
        controller.abort();
        dispose();
        onError(error);
      }
    }
  })();
  return {
    ready,
    stop() {
      if (stopped) return;
      stopped = true;
      controller.abort();
      dispose();
    },
  };
}
