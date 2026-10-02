import { requestBottom } from './transcript-scroll.js';
// Panels settle in one layout pass; preserve keyboard focus without slide effects.
let pendingFocus = 0;
export function preparePanelFocus(selector) {
  cancelAnimationFrame(pendingFocus);
  const panel = document.querySelector(selector);
  const restore = panel?.contains(document.activeElement);
  pendingFocus = requestAnimationFrame(() => {
    pendingFocus = 0;
    if (!panel?.isConnected || !restore) return;
    const button = document.querySelector('.topbar [data-panel-toggle=sidebar]');
    button?.focus({ preventScroll: true });
  });
}

// Reserve exactly the dock's measured height inside the full-height scroll surface.
export function mountComposerDock(root) {
  let height = 0, workspace;
  const update = () => {
    workspace = root.closest('.session-workspace');
    if (!workspace) return;
    const next = Math.ceil(root.getBoundingClientRect().height);
    if (next === height) return;
    const results = workspace.querySelector('.results-panel');
    const follow = results?.classList.contains('sticky-bottom');
    height = next;
    workspace.style.setProperty('--composer-inset', `${height}px`);
    if (follow) requestBottom(results);
  };
  const observer = new ResizeObserver(update);
  observer.observe(root);
  update();
  return () => { observer.disconnect(); workspace?.style.removeProperty('--composer-inset'); };
}
