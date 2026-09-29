import { commands, defaults, fromEvent, validate } from './catalog.mjs';
export const storageKey = 'proteus.shortcuts';
export const mac = /Mac|iPhone|iPad/.test(navigator.platform);
let bindings, error = '';
const handlers = new Map();
function load() {
  try { const raw = localStorage.getItem(storageKey); bindings = raw === null ? defaults() : validate(JSON.parse(raw)); error = ''; }
  catch (reason) { bindings = {}; error = `Не удалось загрузить сочетания: ${reason.message}`; }
}
load();
export const snapshot = () => ({bindings:{...bindings}, error});
export function save(next) {
  validate(next);
  localStorage.setItem(storageKey, JSON.stringify(next));
  bindings = {...next}; error = '';
  window.dispatchEvent(new Event('proteus-shortcuts-change'));
}
window.addEventListener('storage', event => {
  if (event.key === storageKey || event.key === null) { load(); window.dispatchEvent(new Event('proteus-shortcuts-change')); }
});
function keydown(event) {
  if (event.defaultPrevented || document.documentElement.dataset.shortcutRecording) return;
  const binding = fromEvent(event, mac);
  const ordered = [...handlers].sort((a,b) => b[1]-a[1]);
  const dispatch = id => ordered.some(([handler]) => handler(id));
  // Escape dismissal is UI navigation, independent of the stop-answer binding.
  if (binding === 'Escape') {
    const popup = document.querySelector(':popover-open');
    if (popup) { popup.hidePopover(); event.preventDefault(); return; }
    if (dispatch('dismiss-menu')) { event.preventDefault(); event.stopPropagation(); return; }
  }
  const command = commands.find(c => binding && bindings[c.id] === binding);
  // Native window commands win over web navigation regardless of startup order.
  if (command && dispatch(command.id)) { event.preventDefault(); event.stopPropagation(); }

}
export function registerShortcuts(handler, priority = 0) {
  if (!handlers.size) window.addEventListener('keydown', keydown);
  handlers.set(handler, priority);
  return () => { handlers.delete(handler); if (!handlers.size) window.removeEventListener('keydown', keydown); };
}
