import { createPanelRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { theme } from './theme.js';

export function button(label, action, signal) {
  const element=document.createElement('button');element.type='button';element.textContent=label;
  element.addEventListener('click',action,{signal});return element;
}

export function createPanel(record,{services,storage,changed,surfaceOnly=false,createOwned,releaseOwned}) {
  const controller=new AbortController(),{signal}=controller;
  const element=document.createElement('section');element.className='extension-panel';element.dataset.extensionId=record.id;element.dataset.presentation=record.manifest?.presentation??'widget';
  const body=document.createElement('div');body.className='extension-panel-body';
  const error=document.createElement('p');error.className='extension-error';error.setAttribute('role','status');
  const retry=button('Повторить',()=>mount(),signal);retry.hidden=true;element.append(body,error,retry);
  let runtime,panelRoot;
  function mount(){
    releaseOwned?.();runtime?.stop();
    const surface=document.createElement('div');surface.className='extension-panel-content';body.replaceChildren(surface);
    panelRoot=surface.attachShadow({mode:'open'});const style=document.createElement('style');style.textContent=theme;panelRoot.append(style);
    error.textContent=record.error??'';retry.hidden=true;if(record.error||surfaceOnly)return;
    const compact=document.createElement('span').attachShadow({mode:'open'});
    runtime=createPanelRuntime({manifest:record.manifest,root:panelRoot,compact,services,storage:extensionStorage(storage,record.id),
      panel:Object.freeze({open:()=>changed({collapsed:false}),move:location=>changed({location,collapsed:false})}),panels:Object.freeze({create:createOwned}),
      onError(failure){releaseOwned?.();panelRoot.replaceChildren();error.textContent=`Не удалось открыть вкладку: ${failure.message}`;retry.hidden=false;},
    });
  }
  mount();
  return {element,root:panelRoot,signal,update(){},stop(){controller.abort();releaseOwned?.();runtime?.stop();element.remove();}};
}
