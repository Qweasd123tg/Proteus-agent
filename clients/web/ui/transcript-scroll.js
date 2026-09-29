// All layout/render callers share one bottom-follow frame. User intent is
// checked when it runs, including after asynchronous Markdown work.
const frames = new WeakMap();
export function adjustScroll(root, top) {
  // Whole CSS pixels avoid repeatedly assigning fractions that the native
  // scroll position rounds back to the same value.
  const target = Math.round(Math.max(0, Math.min(top, root.scrollHeight - root.clientHeight)));
  // Even an equal assignment can interrupt the browser's ongoing wheel scroll.
  if (Math.abs(root.scrollTop - target) <= .5) return;
  root.scrollTop = target;
  root.dispatchEvent(new CustomEvent('proteus-scroll-adjust', { detail: root.scrollTop }));
}
export function requestBottom(root) {
  if (!root || frames.has(root)) return;
  frames.set(root, requestAnimationFrame(() => {
    frames.delete(root);
    if (root.isConnected && root.clientHeight && root.classList.contains('sticky-bottom')) {
      delete root.dataset.transcriptUserScroll;
      delete root.dataset.transcriptDirection;
      const bottom = Math.max(0, root.scrollHeight - root.clientHeight);
      if (Math.abs(root.scrollTop - bottom) > .5) adjustScroll(root, bottom);
    }
  }));
}
export function cancelBottom(root) {
  const frame = frames.get(root);
  if (frame) cancelAnimationFrame(frame);
  frames.delete(root);
}
