import { icon } from './icons.js';
import { node } from './dom.js';

// One tab strip owns selection and geometry; extension roots stay mounted.
export function createWorkspace(target, { select, close, storage }) {
  const controller = new AbortController(), { signal } = controller;
  const element = node('aside', null, 'tab-workspace'); element.setAttribute('aria-label', 'Боковая панель');
  const header = node('div', null, 'workspace-tabbar'), tabs = node('div', null, 'workspace-tabs');
  tabs.setAttribute('role', 'tablist'); tabs.setAttribute('aria-label', 'Вкладки боковой панели');
  function button(label, glyph, action) {
    const b = node('button'); b.type='button'; b.title=label; b.setAttribute('aria-label',label); b.append(icon(glyph));
    b.addEventListener('click', action, { signal }); return b;
  }
  const picker=node('div',null,'workspace-picker'); picker.setAttribute('popover','auto'); picker.setAttribute('aria-label','Открыть вкладку');
  const add=button('Открыть вкладку','plus',()=>showPicker()); add.className='workspace-add';
  const expand=button('Развернуть панель','expand',()=>{ element.classList.toggle('expanded'); expand.setAttribute('aria-pressed',String(element.classList.contains('expanded'))); });
  const hide=button('Свернуть боковую панель','panel-right',()=>setOpen(false));
  const content=node('div',null,'workspace-tab-content'), empty=node('div',null,'workspace-empty');
  const handle=node('div',null,'workspace-resize'); handle.setAttribute('role','separator'); handle.setAttribute('aria-label','Ширина боковой панели'); handle.setAttribute('aria-orientation','vertical'); handle.tabIndex=0;
  header.append(tabs,add,expand,hide); element.append(header,content,empty,handle,picker); target.append(element);
  let active='', open=false, records=[], signature='', width=560, drag;
  const tabNodes=new Map();
  try { width=Number(storage?.getItem('proteus.ui.workspace.width'))||560; } catch {}
  function setWidth(value) { width=Math.max(320,Math.min(960,value)); element.style.setProperty('--workspace-width',`${width}px`); handle.setAttribute('aria-valuenow',String(width)); }
  function saveWidth() { try { storage?.setItem('proteus.ui.workspace.width',String(width)); } catch {} }
  function showPicker(){ const r=add.getBoundingClientRect();picker.style.left=`${Math.max(8,Math.min(innerWidth-300,r.left))}px`;picker.style.top=`${Math.min(innerHeight-340,r.bottom+6)}px`;picker.showPopover(); }
  function setOpen(value) {
    if(!value&&element.contains(document.activeElement))document.querySelector('[data-workspace-toggle]')?.focus();
    open=value; element.hidden=!open; if(!open) { element.classList.remove('expanded'); expand.setAttribute('aria-pressed','false'); }
    for(const b of document.querySelectorAll('[data-workspace-toggle]')) b.setAttribute('aria-expanded',String(open));
  }
  function reveal(id) { active=id; setOpen(true); update(records); }
  function choose(id) { picker.hidePopover(); select(id); reveal(id); }
  function choices(container) {
    container.replaceChildren();
    for(const record of records.filter(r=>!r.owned)) {
      const b=node('button',record.manifest?.name??record.id); b.type='button'; b.dataset.openTab=record.id;
      container.append(b);
    }
  }
  function update(next) {
    records=next;
    const visible=records.filter(r=>!r.collapsed);
    if(!visible.some(r=>r.id===active)) active=visible.at(-1)?.id??'';
    const ids=new Set(visible.map(r=>r.id));
    for(const [id,tab] of tabNodes) if(!ids.has(id)) { tab.remove(); tabNodes.delete(id); }
    for(const [index,record] of visible.entries()) {
      let tab=tabNodes.get(record.id);
      if(!tab) {
        tab=node('div',null,'workspace-tab'); tab.dataset.tabId=record.id;
        const label=record.manifest?.name??record.id;
        const name=node('button',label,'workspace-tab-name'); name.type='button'; name.title=label; name.setAttribute('role','tab');
        name.id=`workspace-tab-${record.id}`; name.setAttribute('aria-controls',`workspace-view-${record.id}`);
        const remove=node('button');remove.type='button';remove.title=`Закрыть: ${label}`;remove.setAttribute('aria-label',remove.title);remove.append(icon('close'));remove.className='workspace-tab-close';
        tab.append(name,remove); tabNodes.set(record.id,tab);
      }
      if(tabs.children[index]!==tab)tabs.insertBefore(tab,tabs.children[index]??null);
      const selected=active===record.id, name=tab.querySelector('[role=tab]');
      tab.classList.toggle('active',selected); name.setAttribute('aria-selected',String(selected)); name.tabIndex=selected?0:-1;
    }
    for(const child of content.children) { const selected=child.dataset.extensionId===active; child.hidden=!selected; child.inert=!selected; }
    empty.hidden=!!active; content.hidden=!active;
    const nextSignature=records.filter(r=>!r.owned).map(r=>`${r.id}:${r.manifest?.name}`).join('|');
    if(signature!==nextSignature) { signature=nextSignature; choices(picker); choices(empty); }
    setOpen(open);
  }
  for(const area of [picker,empty])area.addEventListener('click',event=>{const b=event.target.closest('[data-open-tab]');if(b)choose(b.dataset.openTab);},{signal});
  tabs.addEventListener('click',event=>{const tab=event.target.closest('[data-tab-id]');if(!tab)return;if(event.target.closest('.workspace-tab-close'))close(tab.dataset.tabId);else reveal(tab.dataset.tabId);},{signal});
  tabs.addEventListener('keydown',event=>{
    if(!event.target.matches('[role=tab]')) return;
    const ids=records.filter(r=>!r.collapsed).map(r=>r.id), index=ids.indexOf(active);
    let id;
    if(event.key==='ArrowRight') id=ids[(index+1)%ids.length];
    else if(event.key==='ArrowLeft') id=ids[(index+ids.length-1)%ids.length];
    else if(event.key==='Home') id=ids[0]; else if(event.key==='End') id=ids.at(-1);
    else if(event.key==='Delete') { event.preventDefault(); close(active); tabs.querySelector('[aria-selected=true]')?.focus(); return; } else return;
    event.preventDefault(); reveal(id); tabs.querySelector('[aria-selected=true]')?.focus();
  },{signal});
  document.addEventListener('click',event=>{
    if(event.target.closest('[data-workspace-toggle]')) setOpen(!open);
    if(event.target.closest('[data-workspace-add]')) { setOpen(true); showPicker(); }
  },{signal});
  document.addEventListener('keydown',event=>{
    if(event.key!=='Escape') return;
    if(picker.matches(':popover-open')) { event.preventDefault(); event.stopImmediatePropagation(); picker.hidePopover(); add.focus(); }
    else if(open&&matchMedia('(max-width:900px)').matches) { event.preventDefault(); event.stopImmediatePropagation(); setOpen(false); document.querySelector('[data-workspace-toggle]')?.focus(); }
  },{signal,capture:true});
  handle.addEventListener('pointerdown',event=>{if(event.button!==0)return;event.preventDefault();drag={x:event.clientX,width:element.getBoundingClientRect().width,id:event.pointerId};},{signal});
  document.addEventListener('pointermove',event=>{if(drag&&event.pointerId===drag.id)setWidth(Math.min(innerWidth-320,drag.width+drag.x-event.clientX));},{signal});
  function end(){if(drag){drag=null;saveWidth();}}
  for(const type of ['pointerup','pointercancel'])document.addEventListener(type,end,{signal});
  window.addEventListener('blur',end,{signal});
  handle.addEventListener('keydown',event=>{if(!['ArrowLeft','ArrowRight'].includes(event.key))return;event.preventDefault();setWidth(width+(event.key==='ArrowLeft'?20:-20));saveWidth();},{signal});
  setWidth(width); setOpen(false);
  return {element,content,update,reveal,stop(){controller.abort();element.remove();}};
}
