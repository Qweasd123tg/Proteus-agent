// Replace native title bubbles without changing Leptos/extension producers.
// The original text remains available as data-ui-tooltip; aria labels survive.
const selector = '[data-ui-tooltip],[title]';
const roots = new WeakSet(), removals = new WeakMap(), labels = new WeakMap();
let bubble, anchor, timer, serial = 0;

function convert(element) {
  const text = element.getAttribute('title');
  if (text === null) return;
  if (text) element.dataset.uiTooltip = text;
  else delete element.dataset.uiTooltip;
  const previous = labels.get(element);
  const unnamed = !element.hasAttribute('aria-label') && !element.hasAttribute('aria-labelledby') &&
    (element.matches('input,select,textarea') || element.matches('button,a') && !element.textContent.trim());
  if (previous && element.getAttribute('aria-label') === previous || unnamed) {
    if (text) element.setAttribute('aria-label', text);
    else element.removeAttribute('aria-label');
    labels.set(element, text);
  }
  if (roots.has(element.getRootNode())) removals.set(element, (removals.get(element) ?? 0) + 1);
  element.removeAttribute('title');
}

function scan(root) {
  if (root instanceof Element && root.hasAttribute('title')) convert(root);
  root.querySelectorAll?.('[title]').forEach(convert);
}

function observe(root) {
  if (roots.has(root)) return;
  // Convert before observing so startup removals do not enter the queue.
  scan(root);
  roots.add(root);
  const observer = new MutationObserver(records => {
    const next = new Map(), changes = new Map();
    // Infer each mutation's new value, including set/remove in the same tick.
    for (let i = records.length - 1; i >= 0; i--) {
      const record = records[i];
      if (record.type !== 'attributes') continue;
      const element = record.target;
      const value = next.has(element) ? next.get(element) : element.getAttribute('title');
      next.set(element, record.oldValue);
      const owned = removals.get(element) ?? 0;
      if (value === null && owned) removals.set(element, owned - 1);
      else if (!changes.has(element)) changes.set(element, value);
    }
    for (const [element, text] of changes) {
      if (text) element.dataset.uiTooltip = text;
      else delete element.dataset.uiTooltip;
      convert(element);
    }
    for (const record of records) {
      if (record.type === 'childList') record.addedNodes.forEach(scan);
    }
    if (anchor && (!anchor.isConnected || !anchor.dataset.uiTooltip)) hideTooltip();
    else if (anchor && bubble?.matches(':popover-open') &&
             bubble.textContent !== anchor.dataset.uiTooltip) {
      bubble.textContent = anchor.dataset.uiTooltip;
      position();
    }
  });
  observer.observe(root, {subtree: true, childList: true, attributes: true,
    attributeFilter: ['title'], attributeOldValue: true});
}

export function hideTooltip() {
  clearTimeout(timer);
  if (anchor && bubble) {
    const ids = (anchor.getAttribute('aria-describedby') ?? '').split(/\s+/)
      .filter(id => id && id !== bubble.id);
    if (ids.length) anchor.setAttribute('aria-describedby', ids.join(' '));
    else anchor.removeAttribute('aria-describedby');
  }
  if (bubble?.matches(':popover-open')) bubble.hidePopover();
  anchor = undefined;
}

function target(event) {
  for (const element of event.composedPath()) {
    if (!(element instanceof Element)) continue;
    const root = element.getRootNode();
    if (root instanceof ShadowRoot) observe(root);
    if (!element.matches(selector)) continue;
    convert(element);
    if (element.dataset.uiTooltip) return element;
  }
}

function position() {
  const viewport = window.visualViewport;
  const left = viewport?.offsetLeft ?? 0, top = viewport?.offsetTop ?? 0;
  const width = viewport?.width ?? innerWidth, height = viewport?.height ?? innerHeight;
  bubble.style.maxWidth = `${Math.min(280, Math.max(1, width - 16))}px`;
  bubble.style.maxHeight = `${Math.max(1, height - 16)}px`;
  const r = anchor.getBoundingClientRect(), size = bubble.getBoundingClientRect();
  const x = Math.max(left + 8, Math.min(r.left + (r.width - size.width) / 2, left + width - size.width - 8));
  const below = r.bottom + 7;
  const y = below + size.height <= top + height - 8 ? below : r.top - size.height - 7;
  bubble.style.left = `${x}px`;
  bubble.style.top = `${Math.max(top + 8, Math.min(y, top + height - size.height - 8))}px`;
}

function show(element, delay) {
  if (!element || element === anchor) return;
  hideTooltip();
  anchor = element;
  timer = setTimeout(() => {
    if (!element.isConnected || !element.dataset.uiTooltip) { hideTooltip(); return; }
    if (!bubble) {
      bubble = document.createElement('div');
      bubble.className = 'ui-tooltip'; bubble.id = `proteus-tooltip-${++serial}`;
      bubble.setAttribute('popover', 'manual'); bubble.setAttribute('role', 'tooltip');
      document.body.append(bubble);
    }
    bubble.textContent = element.dataset.uiTooltip;
    bubble.showPopover(); position();
    const ids = new Set((element.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(Boolean));
    ids.add(bubble.id); element.setAttribute('aria-describedby', [...ids].join(' '));
  }, delay);
}

document.addEventListener('pointerover', event => {
  if (event.pointerType !== 'touch' && !event.buttons) show(target(event), 450);
}, true);
document.addEventListener('focusin', event => show(target(event), 150), true);
document.addEventListener('pointerout', event => {
  if (anchor && !anchor.contains(event.relatedTarget)) hideTooltip();
}, true);
document.addEventListener('focusout', hideTooltip, true);
for (const type of ['pointerdown', 'click', 'dragstart']) document.addEventListener(type, hideTooltip, true);
document.addEventListener('pointermove', event => { if (event.buttons) hideTooltip(); }, true);
document.addEventListener('keydown', event => {
  if (event.key !== 'Escape' || !bubble?.matches(':popover-open')) return;
  event.preventDefault(); event.stopPropagation(); hideTooltip();
}, true);
window.addEventListener('scroll', hideTooltip, true);
window.addEventListener('resize', hideTooltip);
window.addEventListener('blur', hideTooltip);
window.visualViewport?.addEventListener('resize', hideTooltip);
window.visualViewport?.addEventListener('scroll', hideTooltip);
observe(document);
