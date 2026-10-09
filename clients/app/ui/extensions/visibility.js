import { watchLogicalVisibility } from '../ui/visibility.js';

// Panels stay mounted when their tab is hidden. Follow the existing logical
// visibility contract and also pause work when the entire document is hidden.
export function observeVisibility(root, changed, signal) {
  let logical = false;
  let visible;
  const refresh = () => {
    const next = logical && document.visibilityState !== 'hidden';
    if (next === visible) return;
    visible = next;
    changed(visible);
  };
  const stopLogical = watchLogicalVisibility(root, next => {
    logical = next;
    refresh();
  });
  document.addEventListener('visibilitychange', refresh, { signal });
  const stop = () => stopLogical();
  signal.addEventListener('abort', stop, { once: true });
  return stop;
}
