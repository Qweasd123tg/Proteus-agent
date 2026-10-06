// Host actions stay outside extension roots and do not acquire extension services.
export function createWidgetMenu({signal,place,open,hasWorkspace}) {
  let menu,anchor,notice,noticeTimer;
  function close(){if(menu?.matches(':popover-open'))menu.hidePopover();anchor=undefined;}
  function closeNotice(){clearTimeout(noticeTimer);if(notice?.matches(':popover-open'))notice.hidePopover();}
  // A hidden icon leaves no trace in the strip, so say where it went.
  function hidden(id,name,previous,x,y){
    closeNotice();
    if(!notice){notice=document.createElement('div');notice.className='extension-widget-notice choice-surface';notice.setAttribute('popover','manual');notice.setAttribute('role','status');document.body.append(notice);}
    const text=document.createElement('p');text.textContent=`Иконка «${name}» скрыта. Вернуть её можно в настройках расширения.`;
    const actions=document.createElement('div');
    const button=(label,action)=>{const item=document.createElement('button');item.type='button';item.textContent=label;item.addEventListener('click',()=>{closeNotice();action();});return item;};
    actions.append(button('Вернуть',()=>place(id,previous)),button('Настройки',()=>document.dispatchEvent(new CustomEvent('proteus-open-settings-module',{detail:id,cancelable:true}))));
    notice.replaceChildren(text,actions);notice.showPopover();
    const rect=notice.getBoundingClientRect();
    notice.style.left=`${Math.max(8,Math.min(x-rect.width/2,innerWidth-rect.width-8))}px`;
    notice.style.top=`${Math.max(8,Math.min(y-rect.height-12,innerHeight-rect.height-8))}px`;
    noticeTimer=setTimeout(closeNotice,8000);
  }
  function show(event,button){
    event.preventDefault();event.stopPropagation();close();anchor=button;
    if(!menu){menu=document.createElement('div');menu.className='extension-widget-menu choice-surface';menu.setAttribute('popover','manual');menu.setAttribute('role','menu');document.body.append(menu);}
    menu.replaceChildren();
    const id=button.dataset.widgetId,zone=button.closest('[data-widget-zone]')?.dataset.widgetZone??'composer';
    const actions=[],{clientX:x,clientY:y}=event;
    if(hasWorkspace(id))actions.push(['Открыть вкладку',()=>open(id)]);
    actions.push(['В верхнюю панель',()=>place(id,'header')],['Под полем ввода',()=>place(id,'composer')],['Скрыть иконку',()=>{place(id,'hidden');hidden(id,button.dataset.widgetName||id,zone,x,y);}]);
    for(const [label,action]of actions){const item=document.createElement('button');item.type='button';item.className='choice-row';item.setAttribute('role','menuitem');item.textContent=label;item.addEventListener('click',()=>{close();action();});menu.append(item);}
    menu.showPopover();
    const rect=menu.getBoundingClientRect();
    menu.style.left=`${Math.max(8,Math.min(event.clientX,innerWidth-rect.width-8))}px`;
    menu.style.top=`${Math.max(8,Math.min(event.clientY,innerHeight-rect.height-8))}px`;
    menu.firstElementChild.focus();
  }
  document.addEventListener('pointerdown',event=>{const path=event.composedPath();if(menu&&!path.includes(menu))close();if(notice&&!path.includes(notice))closeNotice();},{signal,capture:true});
  document.addEventListener('keydown',event=>{
    if(event.key==='Escape'&&notice?.matches(':popover-open')){event.preventDefault();event.stopImmediatePropagation();closeNotice();return;}
    if(!menu?.matches(':popover-open'))return;
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();const previous=anchor;close();previous?.focus();}
    else if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
      event.preventDefault();const items=[...menu.children],index=items.indexOf(document.activeElement);
      items[event.key==='Home'?0:event.key==='End'?items.length-1:(index+(event.key==='ArrowDown'?1:-1)+items.length)%items.length].focus();
    }
    else if(event.key==='Tab')close();
  },{signal,capture:true});
  window.addEventListener('blur',close,{signal});window.addEventListener('resize',close,{signal});
  signal.addEventListener('abort',()=>{clearTimeout(noticeTimer);menu?.remove();notice?.remove();},{once:true});
  return {show,close};
}
