const positions=['composer','header','hidden'];
const key=id=>`proteus.ui.widget.${id}.position`;
function position(storage,id){
  const saved=storage.getItem(key(id));
  if(saved!==null&&!positions.includes(saved))throw new Error('Неизвестное расположение виджета');
  return saved??'composer';
}

// Move the existing compact roots, keeping each extension runtime alive.
export function createWidgets(storage, reorder) {
  const controller=new AbortController(),{signal}=controller;
  const cancelDrags=[];
  const strips=new Map(positions.filter(p=>p!=='hidden').map(p=>{
    const strip=document.createElement('div');strip.className='extension-widgets';strip.setAttribute('aria-label','Виджеты расширений');
    cancelDrags.push(enableHorizontalReorder(strip,{itemSelector:'.extension-widget',handleSelector:'.extension-widget',id:button=>button.dataset.widgetId,commit:reorder,signal}));
    return [p,{strip,target:null}];
  }));
  let disposed=false,buttons=[];
  function place(){
    if(disposed)return;
    for(const [p,item] of strips){
      if(!item.strip.childElementCount){if(item.strip.parentNode)item.strip.remove();continue;}
      if(!item.target?.isConnected)item.target=document.querySelector(`[data-widget-slot="${p}"]`);
      if(item.target&&item.strip.parentElement!==item.target)item.target.append(item.strip);
    }
  }
  function read(){
    if(disposed)return;
    for(const cancel of cancelDrags)cancel();
    const wanted=new Set(buttons);
    for(const {strip} of strips.values())for(const child of [...strip.children])if(!wanted.has(child))child.remove();
    const offsets=new Map();
    for(const button of buttons){
      let p;try{p=position(storage,button.dataset.widgetId);}catch{p='composer';}
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
    stop(){disposed=true;controller.abort();observer.disconnect();window.removeEventListener('proteus-widgets-position',read);window.removeEventListener('storage',storageChanged);for(const {strip} of strips.values())strip.remove();},
  };
}

export function widgetPlacement(storage,signal,id) {
  const label=document.createElement('label');label.className='extension-widget-placement';label.textContent='Расположение виджета';
  const select=document.createElement('select');select.setAttribute('aria-label','Расположение виджета');select.dataset.widgetPlacement=id;
  for(const [value,text]of [['composer','Под полем ввода'],['header','В верхней панели'],['hidden','Скрыть']]){const option=document.createElement('option');option.value=value;option.textContent=text;select.append(option);}
  const status=document.createElement('span');status.setAttribute('role','status');
  let saved='composer';
  try{saved=position(storage,id);}catch(error){status.textContent=error.message;}
  select.value=saved;
  select.addEventListener('change',()=>{
    try{storage.setItem(key(id),select.value);saved=select.value;status.textContent='';window.dispatchEvent(new Event('proteus-widgets-position'));}
    catch{select.value=saved;status.textContent='Не удалось сохранить расположение';}
  },{signal});
  label.append(select,status);return label;
}
import { enableHorizontalReorder } from './horizontal-reorder.js';
