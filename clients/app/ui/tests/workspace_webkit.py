#!/usr/bin/env python3
"""Native-engine layout and iframe lifetime for two workspace groups."""
import popovers_webkit as harness
harness.PAGE = '''<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css"><link rel="stylesheet" href="/css/layout.css">
<link rel="stylesheet" href="/css/extension-columns.css"><link rel="stylesheet" href="/css/composer-menu.css">
<link rel="stylesheet" href="/css/motion.css">
</head><body><button data-workspace-split>Split</button><div id="target" style="position:relative;height:700px;display:flex"></div>
<script type="module">
import {createWorkspace} from '/extensions/workspace.js';
const store=new Map(), storage={getItem:k=>store.get(k)||null,setItem:(k,v)=>store.set(k,v)};
const board=createWorkspace(document.querySelector('#target'),{storage});
const chat=document.createElement('section'),tools=document.createElement('section'),other=document.createElement('section');
chat.style.overflow='auto';
chat.innerHTML='<textarea>draft</textarea><div style="height:300px"></div><div class="nested" style="height:140px;overflow:auto"><div style="height:800px">nested content</div></div><div style="height:1400px"></div>';
tools.innerHTML='<iframe srcdoc="<input value=state>" style="width:100%;height:100%"></iframe>';
other.textContent='Another tab';
const records=[{id:'client:chat',client:true,manifest:{name:'Чат'},element:chat},{id:'tools',manifest:{name:'Диагностика'},element:tools},{id:'other',manifest:{name:'Other'},element:other}];
const source=board.connect('probe',{select(id){records.find(r=>r.id===id).collapsed=false;source.update(records)},close(id){records.find(r=>r.id===id).collapsed=true;source.update(records)}});source.update(records);
const frame=tools.querySelector('iframe');
const frames=async(count=2)=>{while(count--)await new Promise(requestAnimationFrame)};
const settled=async()=>{await frames();await Promise.allSettled([chat,tools,other].flatMap(root=>root.getAnimations().map(a=>a.finished)));await frames(1)};
frame.addEventListener('load',async()=>{
 try {
  const checks=[],check=(ok,name,detail)=>{if(!ok)throw Error(name+(detail?': '+JSON.stringify(detail):''));checks.push({name});};
  const documentBefore=frame.contentDocument;documentBefore.querySelector('input').value='retained';
  await settled();
  board.reveal('tools');document.querySelector('.workspace-group .workspace-transfer').click();
  await settled();
  check(document.querySelectorAll('.workspace-group:not([hidden])').length===2 && tools.getBoundingClientRect().left>chat.getBoundingClientRect().right && chat.getBoundingClientRect().height>500,'native split geometry');
  const nested=chat.querySelector('.nested');chat.scrollTop=220;nested.scrollTop=130;
  await frames();
  const before=chat.getBoundingClientRect();
  if(chat.scrollTop!==220||nested.scrollTop!==130)throw Error('scroll fixture did not overflow');
  board.reveal('other');await frames(1);
  const exit=chat.getBoundingClientRect(), scrollDuringExit={root:chat.scrollTop,nested:nested.scrollTop};
  const stableExit=chat.hidden && chat.inert && getComputedStyle(chat).position==='absolute' && ['left','top','width','height'].every(key=>Math.abs(exit[key]-before[key])<1);
  if(!stableExit)throw Error('in-place split exit moved: '+JSON.stringify({before,exit,hidden:chat.hidden,inert:chat.inert,position:getComputedStyle(chat).position}));
  board.reveal('client:chat');await settled();
  const rapidScroll=chat.scrollTop===220&&nested.scrollTop===130;const scrollAfterRapid={root:chat.scrollTop,nested:nested.scrollTop};
  chat.scrollTop=220;nested.scrollTop=130;await frames();
  board.reveal('other');await settled();
  if(getComputedStyle(chat).display!=='none')throw Error('closed chat retained painted geometry');
  board.reveal('client:chat');await settled();
  const retainedScroll=chat.scrollTop===220&&nested.scrollTop===130;const scrollAfter={root:chat.scrollTop,nested:nested.scrollTop};
  document.querySelector('.workspace-group[data-group="1"] .workspace-transfer').click();
  check(rapidScroll && retainedScroll && frame.contentDocument===documentBefore && documentBefore.querySelector('input').value==='retained' && chat.querySelector('textarea').value==='draft','native in-place exit, root/nested scroll and document lifetime',{rapidScroll,retainedScroll,scrollDuringExit,scrollAfterRapid,scrollAfter,documentRetained:frame.contentDocument===documentBefore,input:documentBefore.querySelector('input').value,draft:chat.querySelector('textarea').value});
  const handle=document.querySelector('.workspace-resize');handle.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));
  const ratio=board.element.style.getPropertyValue('--workspace-ratio');
  document.querySelector('#target').style.display='none';source.update(records);
  document.querySelector('#target').style.display='flex';source.update(records);
  check(JSON.parse(store.get('proteus.workspace.layout')).ratio<.5 && board.element.style.getPropertyValue('--workspace-ratio')===ratio,'native divider persistence through hidden screen');
  document.querySelector('[data-workspace-split]').click();board.reveal('client:chat');await settled();
  check(document.querySelectorAll('.workspace-group:not([hidden])').length===1 && !chat.hidden && chat.getBoundingClientRect().width>1000,'native merge');
  window.probe=()=>checks;
 } catch(error) {window.probe=()=>{throw error};}
});
</script></body></html>'''
if __name__=='__main__':
    harness.main(label='WebKitGTK split tabs, in-place exit, scroll/iframe lifetime, divider and merge')
