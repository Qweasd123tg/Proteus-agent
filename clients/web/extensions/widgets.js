const KEY='proteus.ui.widgets.position';
export function createWidgets(storage) {
  const strip=document.createElement('div');strip.className='extension-widgets';strip.setAttribute('aria-label','Виджеты расширений');
  let disposed=false,position='composer',target;
  function read(){try{position=storage.getItem(KEY)||'composer';}catch{}target=undefined;place();}
  function place(){
    if(disposed)return;
    strip.hidden=position==='hidden';
    if(!target?.isConnected)target=document.querySelector(`[data-widget-slot="${position}"]`);
    if(target&&strip.parentElement!==target)target.append(strip);
  }
  const observer=new MutationObserver(place);observer.observe(document.body,{childList:true,subtree:true});
  window.addEventListener('proteus-widgets-position',read);window.addEventListener('storage',read);read();
  return {
    update(buttons){
      const wanted=new Set(buttons);for(const child of [...strip.children])if(!wanted.has(child))child.remove();
      buttons.forEach((button,i)=>{if(strip.children[i]!==button)strip.insertBefore(button,strip.children[i]??null);});place();
    },
    stop(){disposed=true;observer.disconnect();window.removeEventListener('proteus-widgets-position',read);window.removeEventListener('storage',read);strip.remove();},
  };
}
export function widgetPlacement(storage,signal) {
  const label=document.createElement('label');label.className='extension-widget-placement';label.textContent='Виджеты расширений';
  const select=document.createElement('select');select.setAttribute('aria-label','Расположение виджетов');
  for(const [value,text]of [['composer','Под полем ввода'],['header','В верхней панели'],['hidden','Скрыть']]){const option=document.createElement('option');option.value=value;option.textContent=text;select.append(option);}
  try{select.value=storage.getItem(KEY)||'composer';}catch{}
  const status=document.createElement('span');status.setAttribute('role','status');
  select.addEventListener('change',()=>{try{storage.setItem(KEY,select.value);status.textContent='';window.dispatchEvent(new Event('proteus-widgets-position'));}catch{status.textContent='Не удалось сохранить расположение';}},{signal});
  label.append(select,status);return label;
}
