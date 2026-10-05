import {selectionButtons} from '../ui/modules/management.js';
import { button } from './panel.js';
import { icon } from './icons.js';
import { enableReorder } from './settings-reorder.js';
import { mountDisclosureMotion } from '../ui/disclosure-motion.js';

function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text) element.textContent = text;
  if (className) element.className = className;
  return element;
}

export function mountExtensionSettings(root, registry) {
  const controller = new AbortController();
  const { signal } = controller;
  root.classList.add('extension-management');
  let rowsController;
  const rowCache=new Map();
  function choose(record){
    document.dispatchEvent(new CustomEvent('proteus-select-settings-module', { detail: record.id }));
  }
  const list = node('div', '', 'extension-list');
  const available = node('div', '', 'extension-available');
  const notice = node('p', '', 'extension-error');
  const announcement = node('span', '', 'extension-reorder-status');announcement.setAttribute('role','status');
  notice.setAttribute('role', 'status');
  const source = node('details', '', 'extension-source');
  source.append(node('summary', 'Добавить по ссылке'));
  const form = node('form', '', 'extension-install');
  const input = node('input');
  input.type = 'url'; input.required = true;
  input.placeholder = 'https://example.com/extension.json';
  input.setAttribute('aria-label', 'URL манифеста расширения');
  const submit = node('button', 'Добавить'); submit.type = 'submit';
  form.append(input, submit); source.append(form);
  const reset = node('details', '', 'extension-reset');
  reset.append(node('summary', 'Восстановить стандартный список'));
  reset.append(node('p', 'Состав и порядок панелей заменятся поставляемым списком. Заметки сохранятся.', 'settings-hint'));
  const restore = button('Восстановить', () => { reset.open = false; void registry.reset(); }, signal);
  reset.append(restore);
  root.append(list, available, source, notice, announcement, reset);
  const resetContent=node('div');
  resetContent.append(...[...reset.children].slice(1));reset.append(resetContent);
  mountDisclosureMotion(source,form,signal);
  mountDisclosureMotion(reset,resetContent,signal);
  enableReorder(list,registry,signal,announcement);
  const unsubscribe = registry.subscribe(() => {
    const { records: allRecords, bundled, notice: message, busy, ready } = registry.state();
    const records=allRecords.filter(r=>r.source==='package');
    const focusKey = document.activeElement?.dataset.controlKey;
    rowsController?.abort(); rowsController = new AbortController();
    const rowSignal = rowsController.signal;
    notice.textContent = message || (!ready && busy ? 'Загрузка расширений…' : '');
    submit.disabled = busy || !ready; restore.disabled = busy;
    available.replaceChildren();
    list.querySelector('.settings-hint')?.parentElement===list&&list.querySelector('.settings-hint').remove();
    for(const [id,item] of rowCache)if(!records.includes(item.record)){item.controller.abort();item.row.remove();rowCache.delete(id);}
    if (ready && !records.length) list.append(node('p', 'Панелей пока нет. Добавьте одну из доступных ниже.', 'settings-hint'));
    records.forEach((record,index) => {
      let cached=rowCache.get(record.id);
      if(!cached){
      const controller=new AbortController(),rowSignal=controller.signal;
      const row = node('div', '', 'extension-choice'); row.dataset.extensionChoice = record.id;
      const label = node('div', '', 'extension-description');
      const text = node('span');
      const name = record.manifest?.name ?? record.id;
      text.append(node('strong', name), node('span', record.error ?? record.manifest?.description, record.error ? 'extension-error' : 'settings-hint'));
      const checkbox = node('input'); checkbox.type = 'checkbox'; checkbox.checked = record.enabled;
      checkbox.className = 'settings-toggle'; checkbox.disabled = busy; checkbox.dataset.controlKey = record.id;
      checkbox.addEventListener('change', () => registry.update(record.id, { enabled: checkbox.checked }), { signal: rowSignal });
      checkbox.setAttribute('aria-label',`Включить: ${name}`);
      const select=button('',()=>choose(record),rowSignal);select.className='extension-select';select.dataset.settingsId=record.id;select.setAttribute('aria-label',`Настроить: ${name}`);select.append(text,icon('chevron-right'));label.append(select,checkbox);
      const grip=button('',()=>{},rowSignal);grip.className='extension-drag-handle';grip.dataset.reorder=record.id;grip.disabled=busy;grip.setAttribute('aria-label',`Переместить: ${name}`);grip.title='Перетащите для изменения порядка · ↑/↓ с клавиатуры';grip.append(icon('grip'));
      row.append(grip,label);
      const actions = node('div', '', 'extension-actions');
      const remove = button('', () => registry.remove(record.id), rowSignal);
      remove.append(icon('close')); remove.title=`Убрать: ${name}`;
      remove.disabled = busy; remove.setAttribute('aria-label', `Убрать: ${name}`);
      actions.append(remove); row.append(actions);
      cached={row,record,controller,checkbox};rowCache.set(record.id,cached);
      }
      cached.row.querySelectorAll('[data-select-slot]').forEach(b=>b.remove());
      selectionButtons(cached.row.querySelector('.extension-actions'),record,registry,rowSignal);
      cached.checkbox.checked=record.enabled;
      for(const control of cached.row.querySelectorAll('button:not([data-select-slot]),input'))control.disabled=busy;
      cached.row.querySelector('[data-settings-id]').disabled=busy||!record.manifest||!!record.error;
      if(list.children[index]!==cached.row)list.insertBefore(cached.row,list.children[index]??null);
    });
    const choices = bundled.filter(item => !records.some(record => record.id === item.id));
    if (choices.length) available.append(node('h3', 'Доступные панели'));
    for (const record of choices) {
      const row = node('div', '', 'extension-choice');
      const description = node('div', '', 'extension-description');
      const text = node('span');
      text.append(node('strong', record.manifest?.name ?? record.id), node('span', record.error ?? record.manifest?.description, 'settings-hint'));
      description.append(text); row.append(description);
      const add = button('Добавить', () => registry.addBundled(record.id), rowSignal);
      add.dataset.extensionAvailable = record.id; add.disabled = busy || !ready || !!record.error;
      add.setAttribute('aria-label', `Добавить: ${record.manifest?.name ?? record.id}`);
      row.append(add); available.append(row);
    }
    if (focusKey) [...root.querySelectorAll('[data-control-key]')].find(item => item.dataset.controlKey === focusKey)?.focus();
  });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (await registry.install(input.value)) input.value = '';
  }, { signal });
  void registry.start();
  return () => { controller.abort(); rowsController?.abort(); for(const item of rowCache.values())item.controller.abort();rowCache.clear(); unsubscribe(); root.replaceChildren(); root.classList.remove('extension-management'); };
}
