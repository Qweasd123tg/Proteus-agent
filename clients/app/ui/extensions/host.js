import { createExtensionRegistry } from './registry.js';
import { createPanel } from './panel.js';
import { createWidgets } from './widgets.js';
import { hasSurface } from './contract.js';
import { createWorkspace } from './workspace.js';

export function mountExtensions(root, services = {}, options = {}) {
  const registry=options.registry??createExtensionRegistry(options), cards=new Map(), owned=new Map();
  const widgets=createWidgets(registry.storage,reorder,{open:id=>update(id,{collapsed:false}),hasWorkspace:id=>hasSurface(registry.state().records.find(r=>r.id===id)?.manifest,'workspace')});
  let stopped=false;
  const clientOwner={id:'client'};
  const all=()=>[...registry.state().records.filter(r=>r.enabled&&hasSurface(r.manifest,'workspace')),...[...owned.values()].map(item=>item.record)];
  const board=options.workspace??createWorkspace(options.target??root,{storage:registry.storage});
  const workspace=board.connect('extensions',{select:id=>update(id,{collapsed:false}),close});
  const notice=document.createElement('p');notice.className='extension-surface-status';notice.setAttribute('role','status');
  // The chat is where a broken built-in selection shows: its selectors are gone.
  const repair=document.createElement('button');repair.type='button';repair.className='btn-primary';repair.textContent='Восстановить встроенные расширения';repair.dataset.builtinRepair='workspace';repair.hidden=true;
  repair.addEventListener('click',()=>registry.resetCore?.());
  root.append(notice,repair);
  const widgetError=event=>{notice.textContent=event.detail;};window.addEventListener('proteus-widgets-error',widgetError);
  function reorder(id,before) {
    const records=registry.state().records,current=records.findIndex(record=>record.id===id);
    if(current<0)return;
    const target=before===null?records.length:records.findIndex(record=>record.id===before);
    if(target<0)return;
    registry.move(id,target-current-(current<target?1:0));
  }
  function update(id,change) {
    const item=owned.get(id); if(item)Object.assign(item.record,change);else registry.update(id,change);
    render();if(change.collapsed===false)workspace.reveal(id);
  }
  function close(id) {
    const item=owned.get(id);
    if(item){owned.delete(id);item.card.stop();item.onClose?.();render();}
    else update(id,{collapsed:true});
  }
  function release(owner) {
    for(const [id,item] of owned)if(item.owner===owner){owned.delete(id);item.card.stop();item.onClose?.();}
    if(!stopped)queueMicrotask(render);
  }
  function createOwned(owner,key,{title,location='right',onClose}) {
    if(stopped||owner!==clientOwner&&!cards.has(owner.id))throw new Error('Расширение закрыто');
    if(!/^[a-z0-9][a-z0-9.-]*$/.test(key)||typeof title!=='string'||!title.trim()||!['left','right'].includes(location)||(onClose!==undefined&&typeof onClose!=='function'))throw new Error('Некорректная вкладка');
    const id=`${owner.id}:${key}`;if(owned.has(id))return owned.get(id).handle;
    const record={id,location,owned:true,enabled:true,collapsed:true,manifest:{name:title,presentation:'panel'}};
    const card=createPanel(record,{surfaceOnly:true,lightContent:owner===clientOwner,changed:change=>update(id,change)});
    const handle=Object.freeze({root:card.root,signal:card.signal,show(){if(owned.get(id)?.record===record)update(id,{collapsed:false});},hide(){if(owned.get(id)?.record===record)update(id,{collapsed:true});},close(){if(owned.get(id)?.record===record)close(id);}});
    owned.set(id,{owner,record,card,handle,onClose});render();return handle;
  }
  function render() {
    if(stopped)return;
    const state=registry.state();notice.textContent=state.notice||(!state.ready?'Загрузка вкладок…':'');notice.dataset.loading=String(!state.notice&&!state.ready);repair.hidden=!state.builtinsInvalid;
    for(const [id,card]of cards)if(!state.records.some(r=>r.id===id&&r.enabled&&r===card.record)){release(card.record);card.stop();cards.delete(id);}
    for(const record of state.records.filter(r=>r.enabled&&(hasSurface(r.manifest,'workspace')||hasSurface(r.manifest,'compact'))))if(!cards.has(record.id)){
      const card=createPanel(record,{services,storage:registry.storage,changed:change=>update(record.id,change),createOwned:(key,spec)=>createOwned(record,key,spec),releaseOwned:()=>release(record)});
      cards.set(record.id,{...card,record});
    }
    for(const record of all()){
      const card=cards.get(record.id)??owned.get(record.id).card;card.update();
      card.element.id=`workspace-view-${record.id}`;card.element.setAttribute('role','tabpanel');card.element.setAttribute('aria-labelledby',`workspace-tab-${record.id}`);

    }
    workspace.update(all().map(record=>({...record,element:(cards.get(record.id)??owned.get(record.id).card).element})));
    widgets.update(state.records.filter(r=>r.enabled&&hasSurface(r.manifest,'compact')).map(r=>cards.get(r.id).compact));
  }
  const disposeClient=options.clientTabs?.(Object.freeze({create:(key,spec)=>createOwned(clientOwner,key,spec)}));
  const unsubscribe=registry.subscribe(render);void registry.start();
  return()=>{window.removeEventListener('proteus-widgets-error',widgetError);stopped=true;unsubscribe();disposeClient?.();release(clientOwner);for(const card of cards.values()){release(card.record);card.stop();}widgets.stop();workspace.stop();if(!options.workspace)board.stop();if(!options.registry)registry.dispose();root.replaceChildren();};
}
