import {highlight, math, diagrams} from './markdown-loaders.js';
import {renderInteractive} from './interactive.js';
import {requestBottom} from './transcript-scroll.js';

const finished=new WeakSet(), queued=new Set();
let running=false, frame=0, diagramId=0;
const nearby=new WeakSet(), observed=new WeakSet();
const viewport=new IntersectionObserver(entries=>{
  for(const entry of entries){
    if(entry.isIntersecting){nearby.add(entry.target);enqueue(entry.target.closest('.message'));}
    else nearby.delete(entry.target);
  }
},{rootMargin:'800px 0px'});
function ready(node){
  if(finished.has(node))return false;
  if(!observed.has(node)){observed.add(node);viewport.observe(node);}
  return nearby.has(node);
}
function enqueue(message) {
  if(!message?.matches('.message')||message.matches('.streaming-message'))return;
  queued.add(message);
  if(!running&&!frame)frame=requestAnimationFrame(drain);
}
function collect(root) {
  if(root.nodeType!==Node.ELEMENT_NODE)return;
  enqueue(root.closest('.message'));
  root.querySelectorAll('.message').forEach(enqueue);
}
function failure(node, error) {
  finished.add(node);
  const status=document.createElement('button');status.type='button';status.className='markdown-render-error';
  status.textContent='Не удалось отобразить · Повторить';status.title=String(error.message||error);
  status.addEventListener('click',()=>{finished.delete(node);status.remove();enqueue(node.closest('.message'));});
  node.parentElement.append(status);
}
async function renderCode(code) {
  if(!code.isConnected||!ready(code))return;
  const language=[...code.classList].find(name=>name.startsWith('language-'))?.slice(9);
  if(!language){finished.add(code);return;}
  if(language==='json-render'){await renderInteractive(code);if(code.isConnected)finished.add(code);return;}
  if(language!=='mermaid'){
    const hljs=await highlight();
    if(!code.isConnected)return;
    if(hljs.getLanguage(language)){code.innerHTML=hljs.highlight(code.textContent,{language,ignoreIllegals:true}).value;code.classList.add('hljs');}
    finished.add(code);return;
  }
  const mermaid=await diagrams(), id='proteus-diagram-'+(++diagramId);
  if(!code.isConnected)return;
  const source=code.textContent;
  // Parse before render: an incomplete/invalid diagram keeps its source code.
  await mermaid.parse(source);
  const {svg}=await mermaid.render(id,source);
  if(!code.isConnected)return;
  const block=code.closest('.code-block'),pre=code.parentElement;
  const diagram=document.createElement('div');diagram.className='markdown-diagram';diagram.innerHTML=svg;
  block.insertBefore(diagram,pre);pre.hidden=true;
  const toggle=document.createElement('button');toggle.type='button';toggle.className='code-source';toggle.textContent='Код';toggle.setAttribute('aria-expanded','false');
  toggle.addEventListener('click',()=>{pre.hidden=!pre.hidden;toggle.setAttribute('aria-expanded',String(!pre.hidden));});
  block.querySelector('.code-actions').prepend(toggle);finished.add(code);
}
async function render(message) {
  let started=performance.now();
  for(const code of message.querySelectorAll('.code-block pre code')) {
    try{await renderCode(code);}catch(error){if(code.isConnected)failure(code,error);}
    if(performance.now()-started>6){await new Promise(requestAnimationFrame);started=performance.now();}
  }
  const formulas=[...message.querySelectorAll('.mathjax-inline,.mathjax-display')].filter(ready);
  if(!formulas.length)return;
  try{
    const engine=await math();
    const connected=formulas.filter(node=>node.isConnected);
    if(!connected.length)return;
    await engine.typesetPromise(connected);
    connected.forEach(node=>finished.add(node));
  }catch(error){for(const node of formulas)if(node.isConnected)failure(node,error);}
}
async function drain() {
  frame=0;running=true;
  const root=document.querySelector('.results-panel');
  const batch=[...queued];queued.clear();
  try{for(const message of batch)if(message.isConnected&&!message.matches('.streaming-message'))await render(message);}
  finally{
    running=false;
    // Do not pull the reader back down if they scrolled away during async work.
    requestBottom(root);
    if(queued.size&&!frame)frame=requestAnimationFrame(drain);
  }
}
const observer=new MutationObserver(records=>{
  for(const record of records){
    if(record.type==='attributes')enqueue(record.target);
    else{
      enqueue(record.target.closest?.('.message'));
      record.addedNodes.forEach(collect);
      if(window.MathJax?.typesetClear)for(const node of record.removedNodes){
        if(node.nodeType===Node.ELEMENT_NODE&&(node.matches('.message,.mathjax-inline,.mathjax-display')||node.querySelector('.mathjax-inline,.mathjax-display')))window.MathJax.typesetClear([node]);
      }
      for(const node of record.removedNodes)if(node.nodeType===Node.ELEMENT_NODE){
        viewport.unobserve(node);
        for(const child of node.querySelectorAll('code,.mathjax-inline,.mathjax-display'))viewport.unobserve(child);
      }
    }
  }
});
observer.observe(document.body,{childList:true,subtree:true,attributes:true,attributeFilter:['class']});
collect(document.body);

document.addEventListener('click',async event=>{
  const button=event.target.closest('.code-copy,.code-wrap');if(!button)return;
  const block=button.closest('.code-block');if(!block)return;
  if(button.matches('.code-wrap')){block.classList.toggle('wrap');button.classList.toggle('active');return;}
  try{
    await navigator.clipboard.writeText(block.querySelector('pre code')?.textContent||'');
    button.textContent='copied';setTimeout(()=>{if(button.isConnected)button.textContent='copy';},1200);
  }catch{button.textContent='Ошибка копирования';}
});
