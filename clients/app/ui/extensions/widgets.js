import { WIDGET_POSITIONS as positions } from './contract.js';
const key=id=>`proteus.ui.widget.${id}.position`;
// The catalog may suggest where a bundled widget starts; the user's choice wins.
function position(storage,id,fallback='composer'){
  const saved=storage.getItem(key(id));
  if(saved!==null&&!positions.includes(saved))throw new Error('Неизвестное расположение виджета');
  return saved??fallback;
}

// Move the existing compact roots, keeping each extension runtime alive.
export function createWidgets(storage, reorder, {open,hasWorkspace}) {
  const controller=new AbortController(),{signal}=controller;
  const cancelDrags=[];
  let dragging=false;
  const menu=createWidgetMenu({signal,place:setPosition,open,hasWorkspace});
  const strips=new Map(positions.filter(p=>p!=='hidden').map(p=>{
    const strip=document.createElement('div');strip.className='extension-widgets';strip.setAttribute('aria-label','Виджеты расширений');
    strip.dataset.widgetZone=p;
    strip.addEventListener('contextmenu',event=>{const button=event.target.closest('.extension-widget');if(button)menu.show(event,button);},{signal});
    cancelDrags.push(enableHorizontalReorder(strip,{itemSelector:'.extension-widget',handleSelector:'.extension-widget',id:button=>button.dataset.widgetId,commit:drop,signal,lists:()=>[...strips.values()].map(item=>item.strip),onStart(){dragging=true;menu.close();place();},onFinish(){dragging=false;place();}}));
    return [p,{strip,target:null}];
  }));
  let disposed=false,buttons=[];
  function place(){
    if(disposed)return;
    for(const [p,item] of strips){
      if(!item.target?.isConnected)item.target=document.querySelector(`[data-widget-slot="${p}"]`);
      item.target?.classList.toggle('widget-drop-target',dragging);
      item.strip.classList.toggle('extension-widget-drop-zone',dragging);
      if(!item.strip.childElementCount&&!dragging){if(item.strip.parentNode)item.strip.remove();continue;}
      if(item.target&&item.strip.parentElement!==item.target)item.target.append(item.strip);
    }
  }
  function setPosition(id,value){
    try{storage.setItem(key(id),value);window.dispatchEvent(new Event('proteus-widgets-position'));}
    catch{window.dispatchEvent(new CustomEvent('proteus-widgets-error',{detail:'Не удалось сохранить расположение иконки'}));}
  }
  function drop(id,before,target){
    try{storage.setItem(key(id),target.dataset.widgetZone);}catch{read();window.dispatchEvent(new CustomEvent('proteus-widgets-error',{detail:'Не удалось сохранить расположение иконки'}));return;}
    reorder(id,before);read();
  }
  function read(){
    if(disposed)return;
    for(const cancel of cancelDrags)cancel();
    const wanted=new Set(buttons);
    for(const {strip} of strips.values())for(const child of [...strip.children])if(!wanted.has(child))child.remove();
    const offsets=new Map();
    for(const button of buttons){
      let p;try{p=position(storage,button.dataset.widgetId,button.dataset.widgetDefault);}catch{p='composer';}
      if(p==='hidden'){button.remove();continue;}
      const strip=strips.get(p).strip,index=offsets.get(p)??0;
      if(strip.children[index]!==button)strip.insertBefore(button,strip.children[index]??null);
      offsets.set(p,index+1);
    }
    place();
  }
  const observer=new MutationObserver(place);observer.observe(document.body,{childList:true,subtree:true});
  const storageChanged=event=>{if(event.key===null||event.key?.startsWith('proteus.ui.widget.'))read();};
  window.addEventListener('proteus-widgets-position',read);window.addEventListener('storage',storageChanged);
  return {
    update(next){buttons=next;read();},
    stop(){disposed=true;controller.abort();observer.disconnect();window.removeEventListener('proteus-widgets-position',read);window.removeEventListener('storage',storageChanged);for(const {strip,target} of strips.values()){strip.remove();target?.classList.remove('widget-drop-target');}},
  };
}

export function widgetPlacement(storage,signal,id,fallback) {
  // One settings row like the other parameters: text on the left, choice on the right.
  const label=document.createElement('label');label.className='settings-row extension-widget-placement';
  const text=document.createElement('span');text.className='settings-label';
  const title=document.createElement('strong');title.textContent='Расположение виджета';
  const hint=document.createElement('span');hint.className='settings-hint';hint.textContent='Где показывать иконку. Её также можно перетащить или скрыть через ПКМ.';
  const select=document.createElement('select');select.setAttribute('aria-label','Расположение виджета');select.dataset.widgetPlacement=id;
  for(const [value,text]of [['composer','Под полем ввода'],['header','В верхней панели'],['hidden','Скрыть']]){const option=document.createElement('option');option.value=value;option.textContent=text;select.append(option);}
  const status=document.createElement('span');status.setAttribute('role','status');
  let saved='composer';
  try{saved=position(storage,id,fallback);}catch(error){status.textContent=error.message;}
  select.value=saved;
  select.addEventListener('change',()=>{
    try{storage.setItem(key(id),select.value);saved=select.value;status.textContent='';window.dispatchEvent(new Event('proteus-widgets-position'));}
    catch{select.value=saved;status.textContent='Не удалось сохранить расположение';}
  },{signal});
  text.append(title,hint,status);label.append(text,select);return label;
}
import { enableHorizontalReorder } from './horizontal-reorder.js';

import { createWidgetMenu } from './widget-menu.js';
