#!/usr/bin/env python3
"""Native-engine layout and iframe lifetime for two workspace groups."""
import popovers_webkit as harness
harness.PAGE = '''<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css"><link rel="stylesheet" href="/css/layout.css">
<link rel="stylesheet" href="/css/extension-columns.css"><link rel="stylesheet" href="/css/composer-menu.css">
</head><body><button data-workspace-split>Split</button><div id="target" style="position:relative;height:700px;display:flex"></div>
<script type="module">
import {createWorkspace} from '/extensions/workspace.js';
import {menu} from '/ui/modules/menu.js';
const store=new Map(), storage={getItem:k=>store.get(k)||null,setItem:(k,v)=>store.set(k,v)};
const board=createWorkspace(document.querySelector('#target'),{storage});
const chat=document.createElement('section'),tools=document.createElement('section');
chat.innerHTML='<textarea>draft</textarea>';tools.innerHTML='<iframe srcdoc="<input value=state>" style="width:100%;height:100%"></iframe>';
const records=[{id:'client:chat',client:true,manifest:{name:'Чат'},element:chat},{id:'tools',manifest:{name:'Диагностика'},element:tools}];
const source=board.connect('probe',{select(id){records.find(r=>r.id===id).collapsed=false;source.update(records)},close(id){records.find(r=>r.id===id).collapsed=true;source.update(records)}});source.update(records);
const frame=tools.querySelector('iframe');
frame.addEventListener('load',()=>{
 const documentBefore=frame.contentDocument;documentBefore.querySelector('input').value='retained';
 window.probe=()=>{
  const checks=[],check=(ok,name)=>{if(!ok)throw Error(name);checks.push({name});};
  board.reveal('tools');document.querySelector('.workspace-group .workspace-transfer').click();
  check(document.querySelectorAll('.workspace-group:not([hidden])').length===2 && tools.getBoundingClientRect().left>chat.getBoundingClientRect().right && chat.getBoundingClientRect().height>500,'native split geometry');
  document.querySelector('.workspace-group[data-group="1"] .workspace-transfer').click();
  check(frame.contentDocument===documentBefore && documentBefore.querySelector('input').value==='retained' && chat.querySelector('textarea').value==='draft','native document lifetime');
  const handle=document.querySelector('.workspace-resize');handle.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));
  check(JSON.parse(store.get('proteus.workspace.layout')).ratio<.5,'native divider persistence');
  document.querySelector('[data-workspace-split]').click();board.reveal('client:chat');
  check(document.querySelectorAll('.workspace-group:not([hidden])').length===1 && !chat.hidden && chat.getBoundingClientRect().width>1000,'native merge');
  return checks;
 };
});
</script></body></html>'''
if __name__=='__main__':
    harness.main(label='WebKitGTK split tabs, iframe lifetime, divider and merge')
