import { createPanelRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { icon } from './icons.js';
import { hasSurface } from './contract.js';
import { theme } from './theme.js';

export function button(label, action, signal) {
  const element=document.createElement('button');element.type='button';element.textContent=label;
  element.addEventListener('click',action,{signal});return element;
}

export function createPanel(record,{services,storage,changed,surfaceOnly=false,lightContent=false,createOwned,releaseOwned}) {
  const controller=new AbortController(),{signal}=controller;
  const element=document.createElement('section');element.className='extension-panel';element.dataset.extensionId=record.id;element.dataset.presentation=record.manifest?.presentation??'widget';
  const body=document.createElement('div');body.className='extension-panel-body';
  const error=document.createElement('p');error.className='extension-error';error.setAttribute('role','status');
  const retry=button('Повторить',()=>mount(),signal);retry.hidden=true;element.append(body,error,retry);
  const workspace=hasSurface(record.manifest,'workspace');
  const compact=button('',()=>{if(workspace)changed({collapsed:false});},signal);compact.className='extension-widget';compact.dataset.widgetId=record.id;compact.title=record.manifest?.name??record.id;compact.setAttribute('aria-label',compact.title);
  const compactSurface=document.createElement('span');compact.append(compactSurface);const compactRoot=compactSurface.attachShadow({mode:'open'});
  const hover=Object.freeze({set(text){if(typeof text!=='string')throw new Error('Подсказка должна быть текстом');compact.dataset.uiTooltipDetails=text;}});
  hover.set(record.manifest?.description??'');
  let runtime,panelRoot;
  function mount(){
    releaseOwned?.();runtime?.stop();
    const surface=document.createElement('div');surface.className='extension-panel-content';body.replaceChildren(surface);
    panelRoot=lightContent?surface:surface.attachShadow({mode:'open'});
    if(!lightContent){const style=document.createElement('style');style.textContent=theme;panelRoot.append(style);}
    error.textContent=record.error??'';retry.hidden=true;
    compactRoot.replaceChildren(icon('modules'));
    if(record.error||surfaceOnly)return;
    runtime=createPanelRuntime({manifest:record.manifest,root:panelRoot,compact:hasSurface(record.manifest,'compact')?compactRoot:undefined,hover,services,storage:extensionStorage(storage,record.id),
      panel:workspace?Object.freeze({open:()=>changed({collapsed:false}),move:location=>changed({location,collapsed:false})}):undefined,panels:Object.freeze({create:createOwned}),
      onError(failure){hover.set(`Не удалось запустить расширение: ${failure.message}`);releaseOwned?.();panelRoot.replaceChildren();error.textContent=`Не удалось открыть вкладку: ${failure.message}`;retry.hidden=false;},
    });
  }
  mount();
  return {element,compact,root:panelRoot,signal,update(){},stop(){controller.abort();releaseOwned?.();runtime?.stop();compact.remove();element.remove();}};
}
