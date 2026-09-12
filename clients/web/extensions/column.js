// Independent extension columns share the shell, never the chat's content slot.
export function createColumn(record, card, storage) {
  const controller = new AbortController(), { signal } = controller;
  const element = document.createElement('aside'); element.className = 'extension-column extension-host';
  element.dataset.columnId = record.id; element.setAttribute('aria-label', record.manifest.name);
  const handle = document.createElement('div'); handle.className = 'extension-column-resize';
  handle.setAttribute('role', 'separator'); handle.setAttribute('aria-orientation', 'vertical'); handle.setAttribute('aria-label', `Ширина: ${record.manifest.name}`); handle.tabIndex = 0;
  element.append(card.element, handle);
  const key = `proteus.ui.column.${record.id}.width`, mobile = matchMedia('(max-width: 900px)'), motion = matchMedia('(prefers-reduced-motion: reduce)');
  let width = record.location === 'right' ? 380 : 280, folded, mobileOpen = false, drag, animation;
  try { const saved = Number(storage.getItem(key)); if (Number.isFinite(saved) && saved >= 180 && saved <= 900) width = saved; } catch {}
  function setWidth(value) { width = Math.max(180, Math.min(900, value)); element.style.setProperty('--column-width', `${width}px`); handle.setAttribute('aria-valuenow', String(Math.round(width))); }
  function save() { try { storage.setItem(key, String(Math.round(width))); } catch {} }
  function update() {
    const next = record.collapsed || (mobile.matches && !mobileOpen);
    element.dataset.location = record.location;
    element.classList.toggle('collapsed', next);
    element.classList.toggle('mobile-open', mobile.matches && !next);
    handle.tabIndex = next ? -1 : 0;
    if (folded !== undefined && folded !== next && element.isConnected && !motion.matches) {
      animation?.cancel();
      animation = card.element.animate([{ opacity: .4, translate: `${record.location === 'left' ? -8 : 8}px 0` }, { opacity: 1, translate: '0 0' }], { duration: 160, easing: 'cubic-bezier(.2,.8,.2,1)' });
      animation.id = 'extension-column';
    }
    folded = next; setWidth(width);
  }
  function end() { if (!drag) return; drag = undefined; setWidth(element.getBoundingClientRect().width); element.classList.remove('resizing'); save(); }
  handle.addEventListener('pointerdown', event => {
    if (event.button !== 0 || mobile.matches) return;
    event.preventDefault(); animation?.cancel();
    const shell = element.closest('.app-layout');
    const occupied = shell ? [...shell.querySelectorAll('.sidebar,.info-panel,.extension-column')].filter(node => node !== element).reduce((sum, node) => sum + node.getBoundingClientRect().width, 0) : 0;
    const maximum = shell ? Math.max(180, shell.clientWidth - occupied - 300) : 900;
    drag = { x: event.clientX, width: element.getBoundingClientRect().width, id: event.pointerId, maximum };
    element.classList.add('resizing');
  }, { signal });
  document.addEventListener('pointermove', event => {
    if (!drag || drag.id !== event.pointerId) return;
    setWidth(Math.min(drag.maximum, drag.width + (event.clientX - drag.x) * (record.location === 'right' ? -1 : 1)));
  }, { signal });
  document.addEventListener('pointerup', end, { signal });
  document.addEventListener('pointercancel', end, { signal });
  window.addEventListener('blur', end, { signal });
  handle.addEventListener('dblclick', () => { setWidth(record.location === 'right' ? 380 : 280); save(); }, { signal });
  handle.addEventListener('keydown', event => {
    if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
    event.preventDefault(); setWidth(width + (event.key === 'ArrowRight' ? 20 : -20) * (record.location === 'right' ? -1 : 1)); save();
  }, { signal });
  mobile.addEventListener('change', () => { mobileOpen = false; end(); animation?.cancel(); update(); }, { signal });
  motion.addEventListener('change', () => animation?.cancel(), { signal });
  document.addEventListener('keydown', event => { if (event.key === 'Escape' && mobile.matches && mobileOpen) { mobileOpen = false; update(); card.element.querySelector('.extension-compact')?.focus(); } }, { signal });
  update();
  return { element, update, reveal() { mobileOpen = true; update(); }, stop() { controller.abort(); animation?.cancel(); element.remove(); } };
}
