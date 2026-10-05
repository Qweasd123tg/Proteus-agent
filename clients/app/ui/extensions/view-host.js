import { viewForSurface } from './contract.js';
import { createViewRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { createViewRoot } from './view-root.js';

// Settings pages and composer slots use the same lazy, retryable mount.
export function mountView(root, record, storage, services, surface) {
  const view = viewForSurface(record.manifest, surface);
  if (!view) throw new Error(`Не объявлена поверхность: ${surface}`);
  const container = document.createElement('div');
  container.className = 'client-module-content';
  root.append(container);
  let runtime, disposed = false;
  const controller = new AbortController();
  function start() {
    runtime?.stop();
    container.replaceChildren();
    const content = createViewRoot(container, view.isolation, 'extension-view-content');
    runtime = createViewRuntime({
      view, root: content.root, surface, services,
      storage: extensionStorage(storage, record.id),
      onError(error) {
        if (disposed) return;
        container.replaceChildren();
        const message = document.createElement('p');
        message.className = 'settings-status'; message.setAttribute('role', 'alert');
        message.textContent = `Не удалось открыть расширение: ${error.message}`;
        const retry = document.createElement('button');
        retry.type = 'button'; retry.textContent = 'Повторить';
        retry.addEventListener('click', start, { signal: controller.signal });
        container.append(message, retry);
      },
    });
  }
  start();
  return () => { disposed = true; controller.abort(); runtime?.stop(); container.remove(); };
}
