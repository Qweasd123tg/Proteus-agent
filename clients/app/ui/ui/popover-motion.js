import { motionAllowed } from './motion.js';
import { exitSnapshot, needsExitSnapshot } from './popover-exit.js';

// display keeps only the exiting pixels alive. The native popover state and
// interaction end immediately, including light dismissal and external hide().
const css = `
[data-popover-motion] {
  opacity:0;
  transition:opacity var(--motion-exit,140ms) var(--motion-ease,ease),
    display var(--motion-exit,140ms) allow-discrete,
    overlay var(--motion-exit,140ms) allow-discrete;
}
[data-popover-motion]:popover-open { opacity:1; transition-duration:var(--popover-enter,var(--motion-surface,240ms)); }
[data-popover-motion]:not(:popover-open) { pointer-events:none!important; }
[data-popover-exiting] { display:var(--popover-display,block)!important; z-index:2147483000!important; }
[data-popover-motion]:not(:popover-open):not([data-popover-exiting]) { transition:none!important; }
[data-popover-snapshot]:not(:popover-open) { display:none!important; transition:none!important; }
@starting-style { [data-popover-motion]:popover-open { opacity:0; } }
[data-popover-motion="off"] { transition:none!important; }
[data-popover-measuring] { opacity:var(--popover-start-opacity,0)!important; visibility:hidden!important; transition:none!important; }
`;
const styled = new WeakSet();

export function popoverMotion(element, { onClose, onExit, anchor, quick = false, exit = true } = {}) {
  const root = element.getRootNode();
  if (!styled.has(root)) {
    const style = document.createElement('style');
    style.textContent = css;
    (root instanceof ShadowRoot ? root : document.head).append(style);
    styled.add(root);
  }
  const controller = new AbortController(), { signal } = controller;
  const initialInert = element.inert;
  let revision = 0, previousAnchor, snapshot;
  if (quick) element.style.setProperty('--popover-enter', 'var(--motion-fast,160ms)');
  const sync = () => {
    const enabled = motionAllowed();
    element.dataset.popoverMotion = enabled ? 'on' : 'off';
    if (!enabled) {
      delete element.dataset.popoverExiting;
      snapshot?.stop();
      snapshot = undefined;
    }
  };
  sync();
  const media = matchMedia('(prefers-reduced-motion: reduce)');
  media.addEventListener('change', sync, { signal });
  window.addEventListener('proteus-motion-change', sync, { signal });
  document.addEventListener('module-hide', event => {
    for (let target = anchor?.(); target; target = target.getRootNode()?.host) {
      if (event.target.contains(target)) {
        if (element.matches(':popover-open')) element.hidePopover();
        break;
      }
    }
  }, { capture: true, signal });
  element.addEventListener('beforetoggle', event => {
    const current = ++revision;
    sync();
    element.inert = event.newState === 'closed';
    if (event.newState !== 'closed') return;
    // Firefox accepts allow-discrete but does not retain display on popover
    // dismissal. Keep only the noninteractive visual surface until fade ends.
    if (exit && motionAllowed()) {
      if (needsExitSnapshot(element)) {
        snapshot = exitSnapshot(element);
        element.dataset.popoverSnapshot = '';
      } else {
        element.style.setProperty('--popover-display', getComputedStyle(element).display);
        element.dataset.popoverExiting = '';
      }
    }
    const exiting = snapshot;
    onClose?.();
    queueMicrotask(async () => {
      if (current !== revision || signal.aborted || element.matches(':popover-open')) return;
      // Flush the closed style before collecting newly created exit transitions.
      getComputedStyle(element).opacity;
      await Promise.allSettled(exiting ? [exiting.finished] : element.getAnimations().map(animation => animation.finished));
      if (current === revision && !signal.aborted && !element.matches(':popover-open')) {
        exiting?.stop();
        if (snapshot === exiting) snapshot = undefined;
        delete element.dataset.popoverExiting;
        delete element.dataset.popoverSnapshot;
        onExit?.();
      }
    });
  }, { signal });
  return {
    show(position) {
      sync();
      const target = anchor?.();
      const resume = (snapshot || element.hasAttribute('data-popover-exiting')) && target === previousAnchor;
      const style = snapshot?.current() || getComputedStyle(element);
      element.style.setProperty('--popover-start-opacity', resume ? style.opacity : '0');
      previousAnchor = target;
      snapshot?.stop();
      snapshot = undefined;
      element.dataset.popoverMeasuring = '';
      delete element.dataset.popoverExiting;
      delete element.dataset.popoverSnapshot;
      element.showPopover();
      position?.();
      // Establish an invisible, correctly positioned first frame before entry.
      getComputedStyle(element).opacity;
      delete element.dataset.popoverMeasuring;
    },
    hide() { if (element.matches(':popover-open')) element.hidePopover(); },
    dispose() {
      controller.abort(); ++revision; snapshot?.stop(); snapshot = undefined;
      if (element.matches(':popover-open')) element.hidePopover();
      for (const key of ['popoverMotion', 'popoverMeasuring', 'popoverExiting', 'popoverSnapshot']) delete element.dataset[key];
      for (const key of ['--popover-start-opacity', '--popover-display', '--popover-enter']) element.style.removeProperty(key);
      element.inert = initialInert;
    },
  };
}
