// Shared keyboard/focus behavior for application and sidebar menus.
import { icon } from '../extensions/icons.js';
import { popoverMotion } from './popover-motion.js';
export function popup(className, label, {manual=false}={}) {
  if (!document.querySelector('link[data-popups]')) {
    const style=document.createElement('link');style.rel='stylesheet';style.href=new URL('./popup.css',import.meta.url).href;style.dataset.popups='';document.head.append(style);
  }
  const element=document.createElement('div');element.className=`ui-popup choice-surface ${className}`;element.setAttribute('popover',manual?'manual':'auto');element.setAttribute('aria-label',label);document.body.append(element);
  let anchor;
  const motion=popoverMotion(element,{anchor:()=>anchor});
  function hide(focus=false){if(element.matches(':popover-open'))element.hidePopover();if(focus&&anchor?.isConnected)anchor.focus({preventScroll:true});}
  function show(content, target, point, focus=true){
    hide();anchor=target;element.replaceChildren(...content);motion.show(()=>{
    const r=target.getBoundingClientRect(),size=element.getBoundingClientRect();
    element.style.left=`${Math.max(8,Math.min(point?.x??r.right+6,innerWidth-size.width-8))}px`;
    element.style.top=`${Math.max(8,Math.min(point?.y??r.top,innerHeight-size.height-8))}px`;
    });
    if(focus)element.querySelector('button,input')?.focus({preventScroll:true});
  }
  element.addEventListener('keydown',event=>{
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();hide(true);return;}
    if(event.key==='Tab'){hide();return;}
    if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key)||event.target.matches('input,textarea'))return;
    event.preventDefault();const buttons=[...element.querySelectorAll('button:not(:disabled)')],i=buttons.indexOf(document.activeElement);
    buttons[event.key==='Home'?0:event.key==='End'?buttons.length-1:(i+(event.key==='ArrowUp'?-1:1)+buttons.length)%buttons.length]?.focus();
  });
  function action(text,glyph,run,{danger=false}={}){
    const button=document.createElement('button');button.type='button';button.className=`ui-menu-item choice-row${danger?' danger':''}`;button.setAttribute('role','menuitem');
    if(glyph)button.append(icon(glyph));const title=document.createElement('span');title.className='choice-title';title.textContent=text;button.append(title);
    button.addEventListener('click',()=>{hide(true);run();});return button;
  }
  const resized=()=>hide();window.addEventListener('resize',resized);
  // Linux can dispatch contextmenu before the opening button is released.
  // Manual menus dismiss on the next press, so that release keeps them open.
  const outside=event=>{if(element.matches(':popover-open')&&!event.composedPath().includes(element))hide();};
  const escape=event=>{if(event.key==='Escape'&&element.matches(':popover-open')){event.preventDefault();event.stopPropagation();hide(true);}};
  if(manual){document.addEventListener('pointerdown',outside,true);document.addEventListener('keydown',escape,true);}
  return {element,show,hide,action,dispose(){hide();motion.dispose();element.remove();window.removeEventListener('resize',resized);if(manual){document.removeEventListener('pointerdown',outside,true);document.removeEventListener('keydown',escape,true);}}};
}
