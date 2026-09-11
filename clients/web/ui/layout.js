// Restore keyboard focus after a dock changes its visible surface.
// No geometry reads, transcript animation or compositor layers during a toggle.
let pendingFocus = 0;
export function preparePanelFocus(selector) {
  cancelAnimationFrame(pendingFocus);
  const panel = document.querySelector(selector);
  if (!panel?.contains(document.activeElement)) return;
  pendingFocus = requestAnimationFrame(() => {
    pendingFocus = 0;
    if (!panel.isConnected) return;
    const button = selector === '.info-panel' && innerWidth <= 900 && !panel.classList.contains('open')
      ? document.querySelector('.info-panel-mobile-toggle')
      : [...panel.querySelectorAll('[data-panel-toggle]')].find(button => !button.closest('[inert]'));
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
    if (follow) results.scrollTop = results.scrollHeight;
  };
  const observer = new ResizeObserver(update);
  observer.observe(root);
  update();
  return () => { observer.disconnect(); workspace?.style.removeProperty('--composer-inset'); };
}
