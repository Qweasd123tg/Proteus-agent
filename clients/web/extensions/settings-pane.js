import { node } from './dom.js';
import { icon } from './icons.js';

// Same side-panel dimensions and immediate geometry as the chat workspace.
export function createSettingsPane(root, signal, onHide) {
  const pane=node('aside',null,'extension-options');pane.hidden=true;pane.setAttribute('aria-label','Настройки расширения');
  const header=node('header',null,'extension-options-header'), title=node('h3'), body=node('div',null,'extension-options-body');
  const hide=node('button');hide.type='button';hide.title='Свернуть настройки расширения';hide.setAttribute('aria-label',hide.title);hide.append(icon('panel-right'));
  header.append(title,hide);pane.append(header,body);
  const handle=node('div',null,'workspace-resize');handle.tabIndex=0;handle.setAttribute('role','separator');handle.setAttribute('aria-label','Ширина настроек расширения');handle.setAttribute('aria-orientation','vertical');pane.append(handle);
  (root.closest('.settings-page')??root).append(pane);
  let width=460,drag;
  try{width=Number(localStorage.getItem('proteus.ui.workspace.width'))||460;}catch{}
  function size(value){width=Math.max(300,Math.min(960,value));pane.style.setProperty('--workspace-width',`${width}px`);handle.setAttribute('aria-valuenow',String(width));}
  function save(){try{localStorage.setItem('proteus.ui.workspace.width',String(width));}catch{}}
  handle.addEventListener('pointerdown',event=>{if(event.button!==0)return;event.preventDefault();drag={id:event.pointerId,x:event.clientX,width:pane.getBoundingClientRect().width};handle.setPointerCapture(event.pointerId);},{signal});
  handle.addEventListener('pointermove',event=>{if(drag?.id===event.pointerId)size(drag.width+drag.x-event.clientX);},{signal});
  for(const type of ['pointerup','pointercancel','lostpointercapture'])handle.addEventListener(type,()=>{if(drag){drag=null;save();}},{signal});
  handle.addEventListener('keydown',event=>{if(!['ArrowLeft','ArrowRight'].includes(event.key))return;event.preventDefault();size(width+(event.key==='ArrowLeft'?20:-20));save();},{signal});
  function collapse(){pane.hidden=true;onHide();}
  hide.addEventListener('click',collapse,{signal});
  document.addEventListener('keydown',event=>{if(event.key==='Escape'&&!pane.hidden&&pane.getBoundingClientRect().width){event.preventDefault();event.stopPropagation();collapse();}},{signal});
  size(width);
  return {element:pane,body,show(name){title.textContent=name;pane.hidden=false;},hide:collapse,remove(){pane.remove();}};
}
