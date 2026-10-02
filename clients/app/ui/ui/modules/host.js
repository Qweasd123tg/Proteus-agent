import { createPanelRuntime } from "../../extensions/runtime.js";
import { extensionStorage } from "../../extensions/storage.js";
export function mountModule(root, record, registry, services, surface) {
  let runtime,
    disposed = false;
  const content = document.createElement("div");
  content.className = "client-module-content";
  root.append(content);
  function start() {
    runtime?.stop();
    content.replaceChildren();
    runtime = createPanelRuntime({
      manifest: record.manifest,
      root: content,
      services,
      storage: extensionStorage(registry.storage, record.id),
      surface,
      onError(error) {
        if (disposed) return;
        content.replaceChildren();
        const message = document.createElement("p");
        message.className = "settings-status";
        message.setAttribute("role", "alert");
        message.textContent = `Не удалось открыть расширение: ${error.message}`;
        const retry = document.createElement("button");
        retry.type = "button";
        retry.textContent = "Повторить";
        retry.addEventListener("click", start);
        content.append(message, retry);
      },
    });
  }
  start();
  return () => {
    disposed = true;
    runtime?.stop();
    root.replaceChildren();
  };
}
export function mountComposerSlot(root, slot, registry, services) {
  let current, stop;
  const unsubscribe = registry.subscribe(() => {
    const next = registry.selected(slot);
    if (next === current) return;
    stop?.();
    root.replaceChildren();
    current = next;
    stop = next ? mountModule(root, next, registry, services, slot) : undefined;
  });
  return () => {
    unsubscribe();
    stop?.();
  };
}
