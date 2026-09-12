// Keep the form control and its bindings; render its picker in the app's theme.
// Delegation also covers controls inside extension ShadowRoots.
let active, serial = 0, pointerSelect;
const single = event => event.composedPath().find(node => node instanceof HTMLSelectElement && !node.multiple && node.size <= 1 && !node.matches(':disabled'));

function open(select) {
  active?.close();
  const controller = new AbortController(), { signal } = controller;
  const menu = document.createElement('div');
  menu.className = 'select-picker'; menu.id = `select-picker-${++serial}`;
  menu.setAttribute('role', 'listbox'); menu.setAttribute('popover', 'manual'); menu.tabIndex = -1;
  menu.setAttribute('aria-label', select.getAttribute('aria-label') || select.labels?.[0]?.textContent || select.title || 'Выбор');
  const root = select.getRootNode();
  (root instanceof ShadowRoot ? root : document.body).append(menu);
  const previousAria = ['aria-controls', 'aria-expanded'].map(name => [name, select.getAttribute(name)]);
  let index = select.selectedIndex, query = '', typedAt = 0;
  const options = [...select.options], rows = [];
  const available = i => options[i] && !options[i].hidden && !options[i].disabled && !options[i].parentElement.disabled && !options[i].parentElement.hidden;
  function choose(i) {
    if (!available(i)) return;
    const changed = select.selectedIndex !== i;
    select.selectedIndex = i;
    close();
    if (!changed) return;
    select.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    select.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
  }
  function highlight(i) {
    index = i;
    rows.forEach((row, n) => row.classList.toggle('focused', n === i));
    menu.setAttribute('aria-activedescendant', rows[i]?.id || '');
    rows[i]?.scrollIntoView({ block: 'nearest' });
  }
  let group;
  options.forEach((option, i) => {
    if (option.parentElement instanceof HTMLOptGroupElement && group !== option.parentElement && !option.parentElement.hidden) {
      group = option.parentElement;
      const heading = document.createElement('div'); heading.className = 'select-picker-group'; heading.textContent = group.label; menu.append(heading);
    }
    const row = document.createElement('div'); row.id = `${menu.id}-${i}`; row.className = 'select-picker-option';
    row.setAttribute('role', 'option'); row.setAttribute('aria-selected', String(i === select.selectedIndex));
    row.setAttribute('aria-disabled', String(!available(i))); row.hidden = option.hidden || option.parentElement.hidden;
    row.textContent = option.label; rows.push(row); menu.append(row);
    row.addEventListener('pointermove', () => { if (available(i)) highlight(i); }, { signal });
    row.addEventListener('click', () => choose(i), { signal });
  });
  function close(focus = true) {
    if (active?.menu !== menu) return;
    controller.abort(); observer.disconnect(); removalObserver.disconnect(); menu.remove(); active = undefined;
    previousAria.forEach(([name, value]) => value === null ? select.removeAttribute(name) : select.setAttribute(name, value));
    if (focus && select.isConnected) select.focus({ preventScroll: true });
  }
  active = { menu, select, close };
  const observer = new MutationObserver(() => close(false));
  const removalObserver = new MutationObserver(() => { if (!select.isConnected || !menu.isConnected || select.matches(':disabled')) close(false); });
  for (let ancestorRoot = root; ancestorRoot; ancestorRoot = ancestorRoot instanceof ShadowRoot ? ancestorRoot.host.getRootNode() : null) {
    removalObserver.observe(ancestorRoot, { childList: true, subtree: true, attributes: true, attributeFilter: ['disabled'] });
  }
  observer.observe(select, { childList: true, subtree: true, attributes: true, attributeFilter: ['disabled', 'hidden', 'label', 'value', 'selected', 'multiple', 'size'] });
  select.setAttribute('aria-controls', menu.id); select.setAttribute('aria-expanded', 'true');
  menu.showPopover?.();
  const rect = select.getBoundingClientRect();
  menu.style.minWidth = `${Math.min(Math.max(rect.width, 170), innerWidth - 16)}px`;
  menu.style.maxWidth = `${innerWidth - 16}px`;
  menu.style.maxHeight = `${Math.max(40, Math.min(320, Math.max(innerHeight - rect.bottom, rect.top) - 16))}px`;
  const size = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(rect.left, innerWidth - size.width - 8))}px`;
  menu.style.top = `${Math.max(8, rect.bottom + size.height + 8 < innerHeight ? rect.bottom + 6 : rect.top - size.height - 6)}px`;
  menu.focus({ preventScroll: true }); highlight(available(index) ? index : options.findIndex((_, i) => available(i)));
  document.addEventListener('pointerdown', event => {
    if (!event.composedPath().includes(menu) && !event.composedPath().includes(select)) close(false);
  }, { capture: true, signal });
  document.addEventListener('focusin', event => { if (!event.composedPath().includes(menu) && !event.composedPath().includes(select)) close(false); }, { signal });
  window.addEventListener('resize', () => close(false), { signal });
  window.addEventListener('scroll', event => { if (!event.composedPath().includes(menu)) close(false); }, { capture: true, signal });
  menu.addEventListener('keydown', event => {
    if (event.key === 'Tab') { close(); return; }
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(index); return; }
    let next = index;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = options.length - 1;
    else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') next += event.key === 'ArrowDown' ? 1 : -1;
    else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey) {
      query = (performance.now() - typedAt < 700 ? query : '') + event.key.toLocaleLowerCase(); typedAt = performance.now();
      next = options.findIndex((option, i) => available(i) && option.label.toLocaleLowerCase().startsWith(query));
    } else return;
    event.preventDefault();
    const step = event.key === 'ArrowUp' || event.key === 'End' ? -1 : 1;
    while (next >= 0 && next < options.length && !available(next)) next += step;
    if (available(next)) highlight(next);
  }, { signal });
}

document.addEventListener('pointerdown', event => {
  const select = single(event);
  pointerSelect = undefined;
  if (!select || event.button !== 0) return;
  event.preventDefault(); pointerSelect = select;
  if (active?.select === select) active.close(); else open(select);
}, true);
document.addEventListener('mousedown', event => { if (single(event)) event.preventDefault(); }, true);
document.addEventListener('click', event => {
  const select = single(event);
  if (!select) { pointerSelect = undefined; return; }
  event.preventDefault();
  if (!event.detail || pointerSelect !== select) { if (active?.select === select) active.close(); else open(select); }
  pointerSelect = undefined;
}, true);
document.addEventListener('pointercancel', () => { pointerSelect = undefined; }, true);
document.addEventListener('keydown', event => {
  const select = single(event);
  if (!select || event.ctrlKey || event.metaKey || !([' ', 'Enter', 'ArrowDown', 'ArrowUp', 'Home', 'End', 'F4'].includes(event.key) || (event.key.length === 1 && !event.altKey))) return;
  pointerSelect = undefined;
  event.preventDefault(); open(select);
  if (event.key === 'Home' || event.key === 'End' || (event.key.length === 1 && event.key !== ' ')) active.menu.dispatchEvent(new KeyboardEvent('keydown', { key: event.key }));
}, true);
