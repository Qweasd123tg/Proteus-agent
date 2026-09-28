import { motionAllowed } from '../ui/motion.js';

// Preview insertion with the existing roots; persist only when the pointer drops.
export function enableHorizontalReorder(list, { itemSelector, handleSelector, id, commit, signal, lists = () => [list], onStart, onFinish }) {
  let drag, suppressClick = false, frame;
  const rows = (target = list) => [...target.children].filter(row => row.matches(itemSelector));
  function finish(save) {
    if (!drag) return;
    const current = drag; drag = null;
    cancelAnimationFrame(frame);
    current.ghost?.remove(); current.row.classList.remove('ui-drag-placeholder');
    if (list.hasPointerCapture(current.pointer)) list.releasePointerCapture(current.pointer);
    if (!current.started) return;
    if (save && current.row.isConnected) {
      const target = current.row.parentElement, order = rows(target), next = order[order.indexOf(current.row) + 1];
      commit(id(current.row), next ? id(next) : null, target);
      current.handle.focus({ preventScroll: true });
    } else {
      for (const [target, order] of current.orders) for (const row of order) if (row.isConnected) target.append(row);
    }
    onFinish?.();
  }
  function preview(source) {
    const copy = source.cloneNode(true);
    function shadows(original, clone) {
      if (original.shadowRoot) {
        const root = clone.attachShadow({ mode: 'open' });
        for (const child of original.shadowRoot.childNodes) root.append(child.cloneNode(true));
      }
      for (let i = 0; i < original.children.length; i++) shadows(original.children[i], clone.children[i]);
    }
    shadows(source, copy);
    for (const el of [copy, ...copy.querySelectorAll('[id]')]) el.removeAttribute('id');
    copy.removeAttribute('data-widget-id'); copy.removeAttribute('data-tab-id');
    copy.classList.add('ui-drag-preview'); copy.setAttribute('aria-hidden', 'true'); copy.inert = true;
    return copy;
  }
  function insert() {
    if (!drag?.started) return;
    const target = lists().find(candidate => {
      const rect = candidate.getBoundingClientRect();
      return drag.lastX >= rect.left - 8 && drag.lastX <= rect.right + 8 && drag.lastY >= rect.top - 12 && drag.lastY <= rect.bottom + 12;
    }) ?? drag.row.parentElement;
    const others = rows(target).filter(row => row !== drag.row);
    let before = others.find(row => {
      const rect = row.getBoundingClientRect();
      return drag.lastX < rect.left + rect.width / 2;
    });
    if (!before) {
      before = others.at(-1)?.nextElementSibling ?? null;
      if (before === drag.row) return;
    }
    if (target === drag.row.parentElement && (before ?? null) === drag.row.nextElementSibling) return;
    const positions = new Map(rows(target).map(row => [row, row.getBoundingClientRect().left]));
    target.insertBefore(drag.row, before ?? null);
    if (motionAllowed()) for (const row of rows(target)) {
      if (row === drag.row) continue;
      const delta = positions.get(row) - row.getBoundingClientRect().left;
      if (Number.isFinite(delta) && delta) {
        for (const animation of row.getAnimations()) animation.cancel();
        row.animate([{ transform: `translateX(${delta}px)` }, { transform: 'translateX(0)' }], { duration: 140, easing: 'ease-out' });
      }
    }
  }
  function scroll() {
    if (!drag?.started) return;
    const target=drag.row.parentElement, rect = target.getBoundingClientRect();
    const direction = drag.lastX < rect.left + 20 ? -1 : drag.lastX > rect.right - 20 ? 1 : 0;
    if (direction) { target.scrollLeft += direction * 8; insert(); }
    frame = requestAnimationFrame(scroll);
  }
  list.addEventListener('pointerdown', event => {
    if (drag && event.pointerId !== drag.pointer) return;
    const handle = event.target.closest(handleSelector);
    if (!handle || handle.disabled || event.button !== 0) return;
    finish(false); suppressClick = false;
    const row = handle.closest(itemSelector), rect = row.getBoundingClientRect();
    drag = { row, handle, rect, orders: new Map(lists().map(target=>[target,[...target.children]])), pointer: event.pointerId, x: event.clientX, y: event.clientY, started: false };
  }, { signal });
  document.addEventListener('pointerdown', event => {
    if (!drag || event.pointerId === drag.pointer) suppressClick = false;
  }, { signal, capture: true });
  document.addEventListener('pointermove', event => {
    if (!drag || event.pointerId !== drag.pointer) return;
    if (!drag.row.isConnected) { finish(false); return; }
    if (!drag.started && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 5) return;
    event.preventDefault();
    if (!drag.started) {
      drag.started = true; suppressClick = true; onStart?.();
      drag.ghost = preview(drag.row);
      Object.assign(drag.ghost.style, { width: `${drag.rect.width}px`, height: `${drag.rect.height}px` });
      document.body.append(drag.ghost); drag.row.classList.add('ui-drag-placeholder');
      list.setPointerCapture(event.pointerId);
      frame = requestAnimationFrame(scroll);
    }
    drag.lastX = event.clientX; drag.lastY = event.clientY;
    drag.ghost.style.left = `${drag.rect.left + event.clientX - drag.x}px`;
    drag.ghost.style.top = `${drag.rect.top + event.clientY - drag.y}px`;
    insert();
  }, { signal, passive: false });
  document.addEventListener('pointerup', event => { if (event.pointerId === drag?.pointer) finish(true); }, { signal });
  document.addEventListener('pointercancel', event => {
    if (event.pointerId === drag?.pointer) { finish(false); suppressClick = false; }
  }, { signal });
  list.addEventListener('lostpointercapture', () => {
    if (drag) { finish(false); suppressClick = false; }
  }, { signal });
  document.addEventListener('click', event => {
    if (suppressClick && event.composedPath().some(element=>element?.matches?.(itemSelector)) && event.detail > 0) { suppressClick = false; event.preventDefault(); event.stopImmediatePropagation(); }
  }, { signal, capture: true });
  list.addEventListener('dragstart', event => event.preventDefault(), { signal });
  list.addEventListener('keydown', event => {
    if (!event.altKey || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
    const handle = event.target.closest(handleSelector); if (!handle) return;
    const row = handle.closest(itemSelector), order = rows(), index = order.indexOf(row);
    const step = event.key === 'ArrowLeft' ? -1 : 1;
    if (index + step < 0 || index + step >= order.length) return;
    event.preventDefault(); event.stopImmediatePropagation();
    commit(id(row), step < 0 ? id(order[index - 1]) : order[index + 2] ? id(order[index + 2]) : null, list);
    handle.focus({ preventScroll: true });
  }, { signal, capture: true });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && drag?.started) { event.preventDefault(); event.stopImmediatePropagation(); finish(false); }
  }, { signal, capture: true });
  window.addEventListener('blur', () => { finish(false); suppressClick = false; }, { signal });
  signal.addEventListener('abort', () => finish(false), { once: true });
  return () => finish(false);
}
