import morphdom from '../vendor/morphdom/morphdom.js';

const states = new WeakMap();

// HTML has already passed the canonical Rust sanitizer. During a stream keep
// the existing DOM: appending text must not reconstruct the complete answer.
export function updateMarkdownFragment(root, html, streaming) {
  const previous = states.get(root);
  states.set(root, { html, streaming });
  if (previous?.html === html) return;
  if (!previous || (!streaming && !previous.streaming)) {
    // A changed settled snapshot invalidates any derived renderer state on its
    // old nodes (MathJax, highlighting, Mermaid and interactive blocks).
    root.innerHTML = html;
    return;
  }
  const next = document.createElement('div');
  next.innerHTML = html;
  morphdom(root, next, {
    childrenOnly: true,
    onBeforeElUpdated(from, to) {
      // Opening details is view state; retain it while the answer grows.
      if (from.tagName === 'DETAILS') to.open = from.open;
      return !from.isEqualNode(to);
    },
    onBeforeElChildrenUpdated(from, to) {
      const oldText = from.firstChild, newText = to.firstChild;
      if (from.childNodes.length === 1 && to.childNodes.length === 1
        && oldText.nodeType === Node.TEXT_NODE && newText.nodeType === Node.TEXT_NODE
        && newText.data.startsWith(oldText.data)) {
        oldText.appendData(newText.data.slice(oldText.data.length));
        return false;
      }
      return true;
    },
  });
}

export function disposeMarkdownFragment(root) { states.delete(root); }
