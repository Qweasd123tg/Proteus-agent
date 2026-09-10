// Layout changes once; the compositor carries the visible column to its new position.
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
let pendingFrame = 0;
const animations = new Set();

export function cancelLayoutMotion() {
  cancelAnimationFrame(pendingFrame);
  pendingFrame = 0;
  for (const animation of animations) animation.cancel();
  animations.clear();
}
reducedMotion.addEventListener('change', cancelLayoutMotion);
window.addEventListener('resize', cancelLayoutMotion);

export function preparePanelMotion(selector) {
  const panel = document.querySelector(selector);
  const restoreFocus = panel?.contains(document.activeElement);
  const elements = [...document.querySelectorAll('.results-panel, .composer-shell, .topbar')];
  const before = elements.map(element => ({ element, rect: element.getBoundingClientRect() }));
  cancelLayoutMotion();
  pendingFrame = requestAnimationFrame(() => {
    pendingFrame = 0;
    if (!panel?.isConnected) return;
    if (!reducedMotion.matches && innerWidth > 900) {
      const style = getComputedStyle(document.documentElement);
      const duration = parseFloat(style.getPropertyValue('--panel-duration'));
      const easing = style.getPropertyValue('--panel-easing').trim();
      for (const { element, rect } of before) {
        if (!element.isConnected) continue;
        const next = element.getBoundingClientRect();
        const dx = element.matches('.topbar') ? rect.left - next.left
          : rect.left + rect.width / 2 - next.left - next.width / 2;
        if (Math.abs(dx) < .5) continue;
        const animation = element.animate([
          { transform: `translateX(${dx}px)` }, { transform: 'none' },
        ], { duration, easing });
        animation.id = 'panel-layout';
        animations.add(animation);
        animation.finished.catch(() => {}).finally(() => animations.delete(animation));
      }
    }
    if (restoreFocus) {
      const button = selector === '.info-panel' && innerWidth <= 900 && !panel.classList.contains('open')
        ? document.querySelector('.info-panel-mobile-toggle')
        : [...panel.querySelectorAll('[data-panel-toggle]')].find(button => !button.closest('[inert]'));
      button?.focus({ preventScroll: true });
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
