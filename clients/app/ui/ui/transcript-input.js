// Follow mode belongs to the transcript's gesture, not to wheel events bubbling
// from scrollable code, tool details or embedded controls.
export function targetsTranscript(root, event, direction = event.deltaY) {
  if (event.defaultPrevented) return false;
  if (event.type === 'wheel' && (event.ctrlKey || event.metaKey || event.shiftKey || !direction)) return false;
  for (const node of event.composedPath()) {
    if (node === root) return true;
    if (!(node instanceof Element)) continue;
    if (node.scrollHeight - node.clientHeight <= 1) continue;
    const style = getComputedStyle(node);
    if (!['auto', 'scroll', 'overlay'].includes(style.overflowY)) continue;
    if (style.overscrollBehaviorY === 'contain' || style.overscrollBehaviorY === 'none') return false;
    if (direction < 0 ? node.scrollTop > 0 : node.scrollTop + node.clientHeight < node.scrollHeight - 1) return false;
  }
  // Keyboard scrolling can target the document while the pointer is over chat.
  return event.type === 'keydown';
}
