import { popup } from './popup.js';
const node=(tag,text)=>{const el=document.createElement(tag);el.textContent=text;return el;};

// DOM data comes from Leptos; mutations go back through its state callback.
export function mountSidebar(root, update) {
  const controller=new AbortController(),{signal}=controller;
  const menu=popup('sidebar-menu','Действия',{manual:true}),card=popup('sidebar-hover','Сведения'),rename=popup('sidebar-rename-popup','Переименовать чат');
  card.element.setAttribute('role','tooltip');
  menu.element.setAttribute('role','menu');
  let timer,hideTimer,hover;
  const desktop=()=>document.documentElement.hasAttribute('data-desktop-chrome');
  const native=action=>document.dispatchEvent(new CustomEvent('proteus-desktop-action',{detail:action}));
  function clearHover(){clearTimeout(timer);clearTimeout(hideTimer);card.hide();hover=null;}
  function showHover(row){
    const lines=(row.dataset.hoverDetail??'').split('\n').filter(Boolean);
    const content=[node('strong',row.dataset.hoverTitle),...lines.slice(0,row.dataset.workspace?2:1).map(text=>node('p',text))];
    card.show(content,row,undefined,false);
  }
  function copy(value){navigator.clipboard.writeText(value).catch(()=>update('error','','Не удалось скопировать путь'));}
  function renameChat(row){
    const form=node('form','');form.className='sidebar-rename';
    const label=node('label','Название чата'),input=document.createElement('input');input.value=row.dataset.hoverTitle;input.required=true;input.maxLength=160;input.setAttribute('aria-label','Название чата');label.append(input);
    const save=node('button','Сохранить');save.type='submit';const cancel=node('button','Отмена');cancel.type='button';cancel.addEventListener('click',()=>rename.hide(true));
    form.append(label,save,cancel);form.addEventListener('submit',event=>{event.preventDefault();if(!input.value.trim())return;update('rename',row.dataset.sessionDir,input.value.trim());rename.hide(true);});
    rename.show([form],row);input.select();
  }
  function openMenu(row,event){
    clearHover();const session=row.dataset.sessionDir;
    const entries=session?[
      menu.action('Открыть чат','chat',()=>row.querySelector('.session-item').click()),
      menu.action(row.dataset.pinned==='true'?'Открепить':'Закрепить','pin',()=>update('pin',session,'')),
      menu.action('Переименовать','edit',()=>renameChat(row)),
      menu.action('Копировать путь сессии','copy',()=>copy(session)),
      node('hr',''),
      menu.action(row.dataset.archived==='true'?'Вернуть из архива':'Архивировать','archive',()=>update('archive',session,'')),
      menu.action('Удалить чат','trash',()=>row.querySelector('[data-delete-session]').click(),{danger:true}),
    ]:[
      menu.action('Новый чат','plus',()=>root.querySelector('[aria-label="Новая сессия"]').click()),
      ...(desktop()?[
        menu.action('Настройки проекта…','settings',()=>native('project')),
        menu.action('Открыть в файловом менеджере','folder',()=>native('folder')),
      ]:[]),
      menu.action('Копировать путь проекта','copy',()=>copy(row.dataset.workspace)),
      node('hr',''),
      menu.action(root.dataset.showArchived==='true'?'Показать текущие чаты':'Показать архив чатов','archive',()=>update('show-archive','','')),
    ];
    menu.show(entries,row.querySelector('[data-sidebar-menu]')??row,event?{x:event.clientX,y:event.clientY}:undefined);
  }
  root.addEventListener('contextmenu',event=>{
    const row=event.target.closest('[data-session-dir],[data-workspace]');if(!row)return;
    event.preventDefault();clearHover();
    if(row.dataset.workspace!==undefined)openMenu(row,event);
  },{signal});
  root.addEventListener('click',event=>{
    const more=event.target.closest('[data-sidebar-menu]');if(more){event.preventDefault();openMenu(more.closest('[data-session-dir],[data-workspace]'));return;}
    if(event.target.closest('[data-app-menu]')&&!desktop()){
      menu.show([menu.action('Настройки','settings',()=>root.querySelector('.settings-link').click())],event.target.closest('[data-app-menu]'));
    }
  },{signal});
  root.addEventListener('keydown',event=>{if(event.key==='ContextMenu'||event.shiftKey&&event.key==='F10'){const row=event.target.closest('[data-workspace]');if(row){event.preventDefault();openMenu(row);}}},{signal});
  root.addEventListener('pointerover',event=>{
    if(event.target.closest('[title],[data-ui-tooltip]')){clearHover();return;}
    const row=event.target.closest('[data-hover-title]');if(!row||row===hover||document.querySelector(':popover-open:not(.sidebar-hover)'))return;
    clearHover();hover=row;timer=setTimeout(()=>{
      if(!row.isConnected||!row.matches(':hover'))return;
      showHover(row);
    },450);
  },{signal});
  root.addEventListener('focusin',event=>{
    if(event.target.closest('[title],[data-ui-tooltip]')){clearHover();return;}
    const row=event.target.closest('[data-hover-title]');if(!row||document.querySelector(':popover-open:not(.sidebar-hover)'))return;
    clearHover();hover=row;timer=setTimeout(()=>{if(row.isConnected&&row.contains(document.activeElement))showHover(row);},150);
  },{signal});
  root.addEventListener('focusout',clearHover,{signal});
  root.addEventListener('keydown',event=>{if(event.key==='Escape'&&card.element.matches(':popover-open')){event.preventDefault();event.stopPropagation();clearHover();}},{signal});
  root.addEventListener('pointerout',event=>{if(hover&&!hover.contains(event.relatedTarget)){clearTimeout(timer);clearTimeout(hideTimer);hideTimer=setTimeout(clearHover,180);}},{signal});
  card.element.addEventListener('pointerenter',()=>clearTimeout(hideTimer),{signal});card.element.addEventListener('pointerleave',()=>{clearTimeout(hideTimer);hideTimer=setTimeout(clearHover,180);},{signal});
  root.addEventListener('scroll',clearHover,{signal,capture:true});
  root.addEventListener('pointerdown',clearHover,{signal});
  root.addEventListener('click',clearHover,{signal});
  return ()=>{controller.abort();clearHover();menu.dispose();card.dispose();rename.dispose();};
}
