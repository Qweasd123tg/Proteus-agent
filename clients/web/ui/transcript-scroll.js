// All layout/render callers share one bottom-follow frame. User intent is
// checked when it runs, including after asynchronous Markdown work.
const frames = new WeakMap();
export function adjustScroll(root, top) {
  root.scrollTop = top;
  root.dispatchEvent(new CustomEvent('proteus-scroll-adjust', { detail: root.scrollTop }));
}
export function requestBottom(root) {
  if (!root || frames.has(root)) return;
  frames.set(root, requestAnimationFrame(() => {
    frames.delete(root);
    if (root.isConnected && root.clientHeight && root.classList.contains('sticky-bottom')) {
      delete root.dataset.transcriptUserScroll;
      delete root.dataset.transcriptDirection;
      adjustScroll(root, root.scrollHeight);
    }
  }));
}
export function cancelBottom(root) {
  const frame = frames.get(root);
  if (frame) cancelAnimationFrame(frame);
  frames.delete(root);
}
