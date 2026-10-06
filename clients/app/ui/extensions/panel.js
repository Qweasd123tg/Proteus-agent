import { createViewRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { icon } from './icons.js';
import { viewForSurface } from './contract.js';
import { createViewRoot } from './view-root.js';

export function button(label, action, signal) {
  const element=document.createElement('button');element.type='button';element.textContent=label;
  element.addEventListener('click',action,{signal});return element;
}

export function createPanel(record,{services,storage,changed,surfaceOnly=false,lightContent=false,layout='fill',createOwned,releaseOwned}) {
  const controller=new AbortController(),{signal}=controller;
  const workspaceView=viewForSurface(record.manifest,'workspace'),compactView=viewForSurface(record.manifest,'compact');
  const element=document.createElement('section');element.className='extension-panel';element.dataset.extensionId=record.id;element.dataset.viewLayout=surfaceOnly?layout:(workspaceView??compactView)?.layout;
  const body=document.createElement('div');body.className='extension-panel-body';
  const error=document.createElement('p');error.className='extension-error';error.setAttribute('role','status');
  const retry=button('Повторить',()=>mount(),signal);retry.hidden=true;element.append(body,error,retry);
  const workspace=surfaceOnly||!!workspaceView;
  const compact=button('',()=>{if(workspace)changed({collapsed:false});},signal);compact.className='extension-widget';compact.dataset.widgetId=record.id;if(record.widget)compact.dataset.widgetDefault=record.widget;compact.title=record.manifest?.name??record.id;compact.setAttribute('aria-label',compact.title);
  const compactSurface=document.createElement('span');compact.append(compactSurface);const compactRoot=compactSurface.attachShadow({mode:'open'});
  const hover=Object.freeze({set(text){if(typeof text!=='string')throw new Error('Подсказка должна быть текстом');compact.dataset.uiTooltipDetails=text;}});
  hover.set(record.manifest?.description??'');
  let runtimes=[],panelRoot;
  function stopViews(){for(const runtime of runtimes)runtime.stop();runtimes=[];}
  function mount(){
    let failed=false;
    releaseOwned?.();stopViews();body.replaceChildren();
    error.textContent=record.error??'';retry.hidden=true;
    compactRoot.replaceChildren(icon('modules'));
    if(surfaceOnly){panelRoot=createViewRoot(body,lightContent?'light':'shadow','extension-panel-content').root;return;}
    if(record.error)return;
    // Combined compact/workspace views share one instance, while separate
    // views receive independent services and cancellation.
    for(const view of new Set([workspaceView,compactView].filter(Boolean))){
      if(failed)break;
      const content=createViewRoot(body,view.isolation,'extension-panel-content');
      if(view!==workspaceView&&workspaceView)content.element.hidden=true;
      if(view===(workspaceView??compactView))panelRoot=content.root;
      const runtime=createViewRuntime({view,root:content.root,surface:view===workspaceView?'workspace':'compact',compact:view===compactView?compactRoot:undefined,hover,services,storage:extensionStorage(storage,record.id),
        panel:view===workspaceView?Object.freeze({open:()=>changed({collapsed:false}),move:location=>changed({location,collapsed:false})}):undefined,panels:Object.freeze({create:createOwned}),
        onError(failure){failed=true;stopViews();hover.set(`Не удалось запустить расширение: ${failure.message}`);releaseOwned?.();body.replaceChildren();error.textContent=`Не удалось открыть вкладку: ${failure.message}`;retry.hidden=false;},
      });
      runtimes.push(runtime);
      if(failed)stopViews();
    }
  }
  mount();
  return {element,compact,root:panelRoot,signal,update(){},stop(){controller.abort();releaseOwned?.();stopViews();compact.remove();element.remove();}};
}
