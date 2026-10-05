import { mountView } from "../../extensions/view-host.js";
export function mountComposerSlot(root, slot, registry, services) {
  let current, stop;
  const unsubscribe = registry.subscribe(() => {
    const next = registry.selected(slot);
    if (next === current) return;
    stop?.();
    root.replaceChildren();
    current = next;
    stop = next ? mountView(root, next, registry.storage, services, slot) : undefined;
  });
  return () => {
    unsubscribe();
    stop?.();
  };
}
