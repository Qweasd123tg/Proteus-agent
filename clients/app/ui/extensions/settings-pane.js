import { node } from './dom.js';
import { icon } from './icons.js';
import { watchViewMotion } from '../ui/view-motion.js';
import { logicallyVisible, watchLogicalVisibility } from '../ui/modules/visibility.js';

// The selected extension's settings occupy the main area beside management.
export function createSettingsPane(root, signal, onHide) {
  const pane=node('section',null,'extension-options');pane.hidden=true;pane.setAttribute('aria-label','Настройки расширения');
  const header=node('header',null,'extension-options-header'), title=node('h3'), body=node('div',null,'extension-options-body');
  const hide=node('button');hide.type='button';hide.title='Вернуться к расширениям';hide.setAttribute('aria-label',hide.title);hide.append(icon('arrow-left'));
  header.append(title,hide);pane.append(header,body);
  (root.closest('.settings-content')??root).append(pane);
  let opened = false;
  function syncVisibility() {
    const hidden = !opened || !logicallyVisible(root);
    if (pane.hidden !== hidden) pane.hidden = hidden;
    if (pane.inert !== hidden) pane.inert = hidden;
  }
  const stopVisibility = watchLogicalVisibility(root, syncVisibility);
  signal.addEventListener('abort', stopVisibility, { once: true });
  const stopMotion=watchViewMotion(pane,{signal});
  function close(){opened=false;syncVisibility();}
  function collapse(){close();onHide();}
  hide.addEventListener('click',collapse,{signal});
  document.addEventListener('keydown',event=>{if(event.key==='Escape'&&!pane.hidden&&pane.getBoundingClientRect().width){event.preventDefault();event.stopPropagation();collapse();}},{signal});
  return {element:pane,body,show(name){title.textContent=name;opened=true;syncVisibility();},close,hide:collapse,remove(){stopVisibility();stopMotion();pane.remove();}};
}
