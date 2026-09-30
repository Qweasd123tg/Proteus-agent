// Host actions stay outside extension roots and do not acquire extension services.
export function createWidgetMenu({signal,place,open,hasWorkspace}) {
  let menu,anchor;
  function close(){if(menu?.matches(':popover-open'))menu.hidePopover();anchor=undefined;}
  function show(event,button){
    event.preventDefault();event.stopPropagation();close();anchor=button;
    if(!menu){menu=document.createElement('div');menu.className='extension-widget-menu choice-surface';menu.setAttribute('popover','manual');menu.setAttribute('role','menu');document.body.append(menu);}
    menu.replaceChildren();
    const id=button.dataset.widgetId;
    const actions=[];
    if(hasWorkspace(id))actions.push(['Открыть вкладку',()=>open(id)]);
    actions.push(['В верхнюю панель',()=>place(id,'header')],['Под полем ввода',()=>place(id,'composer')],['Скрыть иконку',()=>place(id,'hidden')]);
    for(const [label,action]of actions){const item=document.createElement('button');item.type='button';item.className='choice-row';item.setAttribute('role','menuitem');item.textContent=label;item.addEventListener('click',()=>{close();action();});menu.append(item);}
    menu.showPopover();
    const rect=menu.getBoundingClientRect();
    menu.style.left=`${Math.max(8,Math.min(event.clientX,innerWidth-rect.width-8))}px`;
    menu.style.top=`${Math.max(8,Math.min(event.clientY,innerHeight-rect.height-8))}px`;
    menu.firstElementChild.focus();
  }
  document.addEventListener('pointerdown',event=>{if(menu&&!event.composedPath().includes(menu))close();},{signal,capture:true});
  document.addEventListener('keydown',event=>{
    if(!menu?.matches(':popover-open'))return;
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();const previous=anchor;close();previous?.focus();}
    else if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
      event.preventDefault();const items=[...menu.children],index=items.indexOf(document.activeElement);
      items[event.key==='Home'?0:event.key==='End'?items.length-1:(index+(event.key==='ArrowDown'?1:-1)+items.length)%items.length].focus();
    }
    else if(event.key==='Tab')close();
  },{signal,capture:true});
  window.addEventListener('blur',close,{signal});window.addEventListener('resize',close,{signal});
  signal.addEventListener('abort',()=>menu?.remove(),{once:true});
  return {show,close};
}
