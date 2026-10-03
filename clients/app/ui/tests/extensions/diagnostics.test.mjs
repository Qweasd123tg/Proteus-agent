import test from 'node:test';
import assert from 'node:assert/strict';
import { diagnosticsService } from '../../ui/modules/diagnostics.js';

// A small mutation-aware DOM seam: no browser, timers or WASM runtime required.
function fixture(t) {
  const observers = new Set();
  class Observer {
    constructor(callback) { this.callback=callback; this.targets=new Map(); observers.add(this); }
    observe(node, options) { this.targets.set(node,options); }
    disconnect() { this.targets.clear(); }
    notify(node,type) {
      const options=this.targets.get(node);
      if(!options?.[type]||this.queued)return;
      this.queued=true;
      queueMicrotask(()=>{this.queued=false;if(this.targets.size)this.callback();});
    }
  }
  const mutate=(node,type)=>{for(const observer of observers)observer.notify(node,type);};
  class Node extends EventTarget {
    constructor(type=1) { super();this.nodeType=type;this.children=[];this.loads=[];this.contentWindow={}; }
    get isConnected() {return this.nodeType===9||!!(this.parentNode??this.host)?.isConnected;}
    get hidden(){return !!this._hidden;}
    set hidden(value){this._hidden=value;mutate(this,'attributes');}
    append(child){child.remove();this.children.push(child);child.parentNode=this;mutate(this,'childList');}
    remove(){const parent=this.parentNode;if(parent){parent.children=parent.children.filter(node=>node!==this);this.parentNode=null;mutate(parent,'childList');}}
    set src(value){this.loads.push(value);}
    get src(){return this.loads.at(-1);}
  }
  const document=new Node(9),window=new EventTarget(),location={href:'http://client.test/?session_dir=one',origin:'http://client.test'};
  document.createElement=()=>new Node();
  const previous=new Map();
  for(const [key,value] of Object.entries({document,window,location,MutationObserver:Observer})){previous.set(key,globalThis[key]);globalThis[key]=value;}
  t.after(()=>{for(const [key,value] of previous){if(value===undefined)delete globalThis[key];else globalThis[key]=value;}});
  let connection='http://inspector.test/?session_dir=one';
  const subscriptions=new Set(),controller=new AbortController();
  t.after(()=>controller.abort());
  const service=diagnosticsService(()=>connection,callback=>{subscriptions.add(callback);return()=>subscriptions.delete(callback);},controller.signal);
  const settings=new Node();document.append(settings);
  const mount=(view,shadow=false)=>{
    const page=new Node();page.hidden=true;settings.append(page);
    const root=new Node(shadow?11:1);
    if(shadow)root.host=page;else page.append(root);
    const stop=service.mount(view,root);
    return {page,root,frame:root.children[0],stop};
  };
  const select=async(page,items)=>{for(const item of items)item.page.hidden=item.page!==page;await flush();};
  const publish=session=>{connection='http://inspector.test/?session_dir='+session;for(const callback of subscriptions)callback();};
  return {Node,document,window,location,settings,mount,select,publish,subscriptions,controller,observers};
}
const flush=async()=>{await Promise.resolve();await Promise.resolve();};
const session=frame=>new URL(frame.src).searchParams.get('session_dir');

test('three retained diagnostic pages defer hidden session loads and apply only the latest URL on reveal',async t=>{
  const f=fixture(t),items=[];
  for(const [index,view] of ['usage','analysis','architecture'].entries()){
    const item=f.mount(view,index===1);items.push(item);
    assert.equal(item.frame.loads.length,0,'hidden initial page must not load Inspector');
    await f.select(item.page,items);assert.equal(item.frame.loads.length,1);
  }
  f.settings.hidden=true;await flush();f.publish('two');f.publish('three');
  assert.deepEqual(items.map(item=>item.frame.loads.length),[1,1,1]);
  f.settings.hidden=false;await flush();
  assert.deepEqual(items.map(item=>item.frame.loads.length),[1,1,2]);
  assert.equal(session(items[2].frame),'three');
  for(const item of items){await f.select(item.page,items);assert.equal(session(item.frame),'three');assert.equal(item.frame.loads.length,2);}
  f.publish('four');assert.equal(session(items[2].frame),'four','visible page updates synchronously');
  assert.deepEqual(items.map(item=>item.frame.loads.length),[2,2,3]);
  f.settings.hidden=true;await flush();f.settings.hidden=false;await flush();
  assert.equal(items[2].frame.loads.length,3,'unchanged connection preserves the existing document');
  f.controller.abort();await flush();
  assert.equal(f.subscriptions.size,0);
  assert.equal([...f.observers].filter(observer=>observer.targets.size).length,0);
  assert.ok(items.every(item=>!item.frame.parentNode));
});

test('visibility follows a retained ShadowRoot when its host moves, including same-task hide before publication',async t=>{
  const f=fixture(t),item=f.mount('usage',true);await f.select(item.page,[item]);
  const hiddenParent=new f.Node();hiddenParent.hidden=true;f.document.append(hiddenParent);
  hiddenParent.append(item.page);f.publish('two');await flush();
  assert.equal(item.frame.loads.length,1);
  f.settings.append(item.page);await flush();
  assert.equal(item.frame.loads.length,2);assert.equal(session(item.frame),'two');
  f.settings.hidden=true;f.publish('three');
  assert.equal(item.frame.loads.length,2,'publication must inspect logical ancestors before the observer runs');
  item.stop();item.stop();f.settings.hidden=false;f.publish('four');await flush();
  assert.equal(item.frame.loads.length,2);assert.equal(f.subscriptions.size,0);
});

test('an old retained Inspector link opens its actual session rather than the new parent session',async t=>{
  const f=fixture(t),item=f.mount('usage');await f.select(item.page,[item]);
  let navigations=0;f.document.addEventListener('proteus-client-navigation',()=>navigations++);
  const message=()=>{const event=new Event('message');Object.assign(event,{source:item.frame.contentWindow,origin:'http://inspector.test',data:{type:'proteus-open-chat',href:'http://client.test/?session_dir=one'}});f.window.dispatchEvent(event);};
  message();assert.equal(navigations,1,'the current session keeps in-app navigation');navigations=0;
  f.settings.hidden=true;await flush();f.publish('two');f.location.href='http://client.test/?session_dir=two';message();
  assert.equal(navigations,0);assert.equal(new URL(f.location.href).searchParams.get('session_dir'),'one');
  assert.equal(new URL(f.location.href).searchParams.get('workspace_view'),'chat');
  f.controller.abort();f.location.href='http://client.test/?session_dir=two';message();
  assert.equal(new URL(f.location.href).searchParams.get('session_dir'),'two','aborted frame must not navigate');
});
