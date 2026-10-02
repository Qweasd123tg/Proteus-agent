import {commands, defaults, fromEvent, label, rebind} from './catalog.mjs';
import {mac, save, snapshot} from './runtime.js';

export function mountShortcutSettings(root) {
  const controller = new AbortController(), options = {signal:controller.signal};
  root.innerHTML = `<div class="shortcut-toolbar"><input type="search" aria-label="Найти команду" placeholder="Найти команду или сочетание"><button type="button" data-reset-shortcuts>Сбросить все</button></div><p class="settings-hint">Нажмите сочетание, затем введите новое. Esc отменяет запись. Сочетания действуют внутри приложения; системные команды могут иметь приоритет.</p><p class="settings-status" role="status" aria-live="polite"></p><div class="shortcut-list"></div>`;
  const search = root.querySelector('input'), status = root.querySelector('[role=status]'), list = root.querySelector('.shortcut-list');
  let recording = null, lastFocus = null;
  function cancel() {
    recording = null; delete document.documentElement.dataset.shortcutRecording;
    render(); lastFocus && list.querySelector(`[data-bind="${lastFocus}"]`)?.focus();
  }
  function commit(next) {
    try { save(next); cancel(); status.textContent = 'Сохранено на этом устройстве'; }
    catch (error) { status.textContent = `Не сохранено: ${error.message}`; }
  }
  function render() {
    const {bindings, error} = snapshot();
    if (error) status.textContent = error;
    list.replaceChildren();
    const query = search.value.trim().toLocaleLowerCase();
    for (const command of commands) {
      if (!`${command.label} ${command.group} ${label(bindings[command.id],mac)}`.toLocaleLowerCase().includes(query)) continue;
      const row = document.createElement('div'); row.className = 'shortcut-row';
      const caption = document.createElement('span'); caption.className = 'settings-label';
      const title = document.createElement('strong'); title.textContent = command.label;
      const hint = document.createElement('span'); hint.className = 'settings-hint'; hint.textContent = command.group + (command.native ? ' · Только desktop' : '');
      caption.append(title,hint);
      const controls = document.createElement('div'); controls.className = 'shortcut-controls';
      const bind = document.createElement('button'); bind.type = 'button'; bind.dataset.bind = command.id;
      bind.textContent = recording === command.id ? 'Нажмите сочетание…' : label(bindings[command.id],mac);
      bind.setAttribute('aria-label', `Сочетание: ${command.label}`); bind.setAttribute('aria-pressed', String(recording === command.id));
      bind.addEventListener('click', () => { recording = command.id; lastFocus = command.id; document.documentElement.dataset.shortcutRecording = 'true'; status.textContent = 'Нажмите сочетание. Esc — отмена.'; render(); list.querySelector(`[data-bind="${command.id}"]`).focus(); });
      const clear = document.createElement('button'); clear.type = 'button'; clear.textContent = 'Убрать'; clear.dataset.clear = command.id; clear.disabled = !bindings[command.id] || !!error;
      clear.setAttribute('aria-label', `Убрать сочетание: ${command.label}`);
      clear.addEventListener('click', () => commit(rebind(snapshot().bindings,command.id,null)));
      const reset = document.createElement('button'); reset.type = 'button'; reset.textContent = 'Сброс'; reset.dataset.reset = command.id;
      reset.setAttribute('aria-label', `Сбросить сочетание: ${command.label}`);
      reset.addEventListener('click', () => { try { commit(rebind(snapshot().bindings,command.id,command.binding)); } catch(error) {status.textContent=error.message;} });
      controls.append(bind,clear,reset); row.append(caption,controls); list.append(row);
    }
    if (!list.children.length) list.textContent = 'Команды не найдены';
  }
  window.addEventListener('keydown', event => {
    if (!recording) return;
    event.preventDefault(); event.stopImmediatePropagation();
    if (event.isComposing || event.repeat) return;
    if (event.key === 'Escape') { cancel(); status.textContent = 'Запись отменена'; return; }
    if (event.key === 'Tab') { cancel(); status.textContent = 'Запись отменена'; return; }
    const binding = fromEvent(event,mac);
    if (!binding) return;
    try { commit(rebind(snapshot().bindings,recording,binding)); }
    catch(error) { status.textContent = error.message; }
  }, {...options,capture:true});
  document.addEventListener('pointerdown', event => { if(recording && !event.target.closest('[data-bind]')) cancel(); }, {...options,capture:true});
  root.closest('.settings-section')?.addEventListener('module-hide', () => {if(recording)cancel();}, options);
  window.addEventListener('blur', cancel, options);
  window.addEventListener('proteus-shortcuts-change', render, options);
  search.addEventListener('input',render,options);
  root.querySelector('[data-reset-shortcuts]').addEventListener('click', () => commit(defaults()),options);
  render();
  return () => { controller.abort(); delete document.documentElement.dataset.shortcutRecording; root.replaceChildren(); };
}
