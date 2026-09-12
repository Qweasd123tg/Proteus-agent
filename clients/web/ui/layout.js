// Only animate the surface entering the dock. Layout width changes once.
let pendingFocus = 0;
const animations = new Set();
const motion = matchMedia('(prefers-reduced-motion: reduce)');
function cancelMotion() { for (const animation of animations) animation.cancel(); animations.clear(); }
window.addEventListener('resize', cancelMotion);
document.addEventListener('mousedown', event => { if (event.target.closest?.('.sidebar-resize-handle,.info-panel-resize-handle,.chat-resize-handle')) cancelMotion(); });
motion.addEventListener('change', cancelMotion);
export function preparePanelFocus(selector) {
  cancelAnimationFrame(pendingFocus); cancelMotion();
  const panel = document.querySelector(selector);
  const restore = panel?.contains(document.activeElement);
  pendingFocus = requestAnimationFrame(() => {
    pendingFocus = 0;
    if (!panel?.isConnected) return;
    const mobileClosed = selector === '.info-panel' && innerWidth <= 900 && !panel.classList.contains('open');
    const button = mobileClosed ? document.querySelector('.info-panel-mobile-toggle')
      : [...panel.querySelectorAll('[data-panel-toggle]')].find(button => !button.closest('[inert]'));
    if (restore) button?.focus({ preventScroll: true });
    if (motion.matches || mobileClosed) return;
    const surfaces = [...panel.querySelectorAll('.sidebar-surface:not([inert]),.sidebar-rail-surface:not([inert]),.info-panel-surface:not([inert]),.info-panel-rail-surface:not([inert]),[data-extension-location]')];
    for (const surface of surfaces) {
      const animation = surface.animate([{ opacity: .45, translate: `${selector === '.sidebar' ? -8 : 8}px 0` }, { opacity: 1, translate: '0 0' }], { duration: 160, easing: 'cubic-bezier(.2,.8,.2,1)' });
      animation.id = 'panel-surface'; animations.add(animation);
      animation.finished.catch(() => {}).finally(() => animations.delete(animation));
    }
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
