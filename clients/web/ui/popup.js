// Shared keyboard/focus behavior for application and sidebar menus.
import { icon } from '../extensions/icons.js';
export function popup(className, label) {
  if (!document.querySelector('link[data-popups]')) {
    const style=document.createElement('link');style.rel='stylesheet';style.href=new URL('./popup.css',import.meta.url).href;style.dataset.popups='';document.head.append(style);
  }
  const element=document.createElement('div');element.className=`ui-popup ${className}`;element.setAttribute('popover','auto');element.setAttribute('aria-label',label);document.body.append(element);
  let anchor;
  function hide(focus=false){if(element.matches(':popover-open'))element.hidePopover();if(focus&&anchor?.isConnected)anchor.focus({preventScroll:true});}
  function show(content, target, point, focus=true){
    hide();anchor=target;element.replaceChildren(...content);element.showPopover();
    const r=target.getBoundingClientRect(),size=element.getBoundingClientRect();
    element.style.left=`${Math.max(8,Math.min(point?.x??r.right+6,innerWidth-size.width-8))}px`;
    element.style.top=`${Math.max(8,Math.min(point?.y??r.top,innerHeight-size.height-8))}px`;
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
    const button=document.createElement('button');button.type='button';button.className=`ui-menu-item${danger?' danger':''}`;button.setAttribute('role','menuitem');
    if(glyph)button.append(icon(glyph));button.append(document.createTextNode(text));
    button.addEventListener('click',()=>{hide(true);run();});return button;
  }
  const resized=()=>hide();window.addEventListener('resize',resized);
  return {element,show,hide,action,dispose(){hide();element.remove();window.removeEventListener('resize',resized);}};
}
