// Placement belongs to the host, including compact and extension-owned panels.
let active;
let serial = 0;

export function attachPanelMenu(header, record, changed, signal) {
  let dismiss;
  function close() { dismiss?.(); }
  function open(trigger, point) {
    active?.();
    const controller = new AbortController();
    const menu = document.createElement('div');
    menu.className = 'select-picker extension-context-menu';
    menu.id = `extension-context-menu-${++serial}`;
    menu.setAttribute('role', 'menu');
    menu.setAttribute('aria-label', `Расположение: ${record.manifest?.name ?? record.id}`);
    menu.setAttribute('popover', 'manual');
    menu.tabIndex = -1;
    const rows = [];
    let index = record.location === 'left' ? 0 : 1;
    function highlight(next) {
      index = next;
      rows.forEach((row, i) => row.classList.toggle('focused', i === index));
      menu.setAttribute('aria-activedescendant', rows[index].id);
    }
    function choose(next) {
      finish(false);
      const location = next === 0 ? 'left' : 'right';
      if (location !== record.location) changed({ location, collapsed: false });
      const target = trigger.getClientRects().length ? trigger : header.querySelector('.extension-compact');
      if (target.isConnected) target.focus({ preventScroll: true });
    }
    function finish(focus = true) {
      controller.abort();
      menu.remove();
      dismiss = undefined;
      if (active === finish) active = undefined;
      if (focus && trigger.isConnected) trigger.focus({ preventScroll: true });
    }
    for (const [i, label] of ['Слева', 'Справа'].entries()) {
      const row = document.createElement('div');
      row.id = `${menu.id}-${i}`;
      row.className = 'select-picker-option';
      row.textContent = label;
      row.setAttribute('role', 'menuitemradio');
      row.setAttribute('aria-checked', String(i === index));
      row.dataset.location = i === 0 ? 'left' : 'right';
      row.addEventListener('pointermove', () => highlight(i), { signal: controller.signal });
      row.addEventListener('click', () => choose(i), { signal: controller.signal });
      rows.push(row); menu.append(row);
    }
    dismiss = () => finish(false);
    active = finish;
    document.body.append(menu);
    menu.showPopover?.();
    const rect = trigger.getBoundingClientRect();
    const x = point?.x ?? rect.left, y = point?.y ?? rect.bottom;
    menu.style.minWidth = `${Math.min(170, innerWidth - 16)}px`;
    menu.style.maxWidth = `${innerWidth - 16}px`;
    menu.style.maxHeight = `${innerHeight - 16}px`;
    const size = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(8, Math.min(x, innerWidth - size.width - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(y, innerHeight - size.height - 8))}px`;
    menu.focus({ preventScroll: true }); highlight(index);
    document.addEventListener('pointerdown', event => {
      if (!event.composedPath().includes(menu)) finish(false);
    }, { capture: true, signal: controller.signal });
    document.addEventListener('focusin', event => {
      if (!event.composedPath().includes(menu)) finish(false);
    }, { signal: controller.signal });
    window.addEventListener('resize', () => finish(false), { signal: controller.signal });
    window.addEventListener('blur', () => finish(false), { signal: controller.signal });
    window.addEventListener('scroll', event => {
      if (!event.composedPath().includes(menu)) finish(false);
    }, { capture: true, signal: controller.signal });
    menu.addEventListener('keydown', event => {
      if (event.key === 'Tab') { finish(); return; }
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); finish(); return; }
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(index); return; }
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      highlight(event.key === 'Home' ? 0 : event.key === 'End' ? 1 : 1 - index);
    }, { signal: controller.signal });
  }
  function trigger(event) {
    const title = header.querySelector('.extension-panel-title');
    return event.target.closest('button') ?? (title.getClientRects().length ? title : header.querySelector('.extension-compact'));
  }
  header.addEventListener('contextmenu', event => {
    event.preventDefault();
    open(trigger(event), event.button === 2 ? { x: event.clientX, y: event.clientY } : undefined);
  }, { signal });
  header.addEventListener('keydown', event => {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return;
    event.preventDefault(); event.stopPropagation(); open(trigger(event));
  }, { signal });
  signal.addEventListener('abort', close, { once: true });
  return { close };
}
