import { node } from './dom.js';
import { icon } from './icons.js';

// Extension options are a detail page inside Settings, not another sidebar.
export function createSettingsPane(root, signal, onHide) {
  const pane=node('section',null,'extension-options');pane.hidden=true;pane.setAttribute('aria-label','Настройки расширения');
  const header=node('header',null,'extension-options-header'), title=node('h3'), body=node('div',null,'extension-options-body');
  const hide=node('button');hide.type='button';hide.title='Вернуться к модулям';hide.setAttribute('aria-label',hide.title);hide.append(icon('arrow-left'));
  header.append(title,hide);pane.append(header,body);
  (root.closest('.settings-content')??root).append(pane);
  function collapse(){pane.hidden=true;onHide();}
  hide.addEventListener('click',collapse,{signal});
  document.addEventListener('keydown',event=>{if(event.key==='Escape'&&!pane.hidden&&pane.getBoundingClientRect().width){event.preventDefault();event.stopPropagation();collapse();}},{signal});
  return {element:pane,body,show(name){title.textContent=name;pane.hidden=false;},hide:collapse,remove(){pane.remove();}};
}
