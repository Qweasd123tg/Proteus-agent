import { node, icon } from '../dom.js';
import { createPreview } from './preview.js';
import { observeVisibility } from '../visibility.js';

export function mount({ root, compact, services, signal }) {
  icon(compact, 'folder');
  root.append(node('style', `
    :host{display:flex!important;flex-direction:column;height:100%;min-height:0;overflow:hidden}
    .toolbar{display:flex;align-items:center;justify-content:flex-end;flex:none;padding:4px 8px;border-bottom:1px solid var(--border-subtle,#333);font-size:11px;color:var(--text-muted,#aaa)}
    .toolbar button{display:flex;align-items:center;padding:3px 6px;font-size:12px}.toolbar svg{width:16px;height:16px}.tree{flex:1;min-height:0;overflow:auto;padding:4px 0;outline:none}
    .row{display:flex;align-items:center;justify-content:flex-start;gap:5px;width:100%;height:25px;min-height:25px;box-sizing:border-box;border:0;border-radius:7px;padding:0 8px;text-align:left;background:transparent;color:inherit;font:12px var(--font-mono,monospace);white-space:nowrap;cursor:pointer}
    .row:hover{background:var(--bg-hover,#333)}.row.active{background:var(--bg-active,var(--bg-hover,#333))}.row:focus-visible{outline:1px solid var(--accent,#8aaaff);outline-offset:-1px}.row:disabled{opacity:.45;cursor:default}
    .row svg{width:14px;height:14px;flex:none}.row .chevron{width:10px;height:10px;transition:transform var(--motion-fast,100ms)}.row[aria-expanded=true] .chevron{transform:rotate(90deg)}
    @media(prefers-reduced-motion:reduce){.row .chevron{transition:none}}
    .file-type-rust{color:#d9a18b}.file-type-js{color:#d8c979}.file-type-json{color:#c7bd84}.file-type-md{color:#86b4d4}
    .git-added .label,.git-untracked .label{color:#8fc99b}.git-modified .label,.git-renamed .label{color:#d8bc79}.git-deleted .label,.git-conflict .label{color:#df9292}.git-deleted .label{text-decoration:line-through}.git-mark{margin-left:auto;flex:none;font-size:10px;color:var(--text-muted,#aaa)}
    .label{overflow:hidden;text-overflow:ellipsis}.spacer{width:10px;flex:none}.status{margin:6px 10px;font-size:12px;overflow-wrap:anywhere;color:var(--text-muted,#aaa)}
  `));
  const toolbar=node('div',null,'toolbar'), refresh=node('button');
  refresh.append(shape('refresh'));
  refresh.type='button'; refresh.title='Обновить дерево'; refresh.setAttribute('aria-label','Обновить дерево');
  const filter=node('input'); filter.type='search'; filter.placeholder='Фильтрация файлов…'; filter.setAttribute('aria-label','Фильтрация файлов'); toolbar.append(filter,refresh);
  let filterFrame;
  function applyFilter(activeRow=root.activeElement?.closest?.('.row'),hadFocus=activeRow&&tree.contains(activeRow)){
    filterFrame=undefined;
    const query=filter.value.toLowerCase();
    const allRows=[...tree.querySelectorAll('.row')];
    for(const row of allRows){
      const hidden=!!query&&!row.textContent.toLowerCase().includes(query);
      if(row.hidden!==hidden)row.hidden=hidden;
    }
    const rows=allRows.filter(row=>!row.disabled&&!row.hidden);
    const target=rows.find(row=>row===activeRow)||rows.find(row=>row.dataset.path===focused)||rows[0];
    if(target)focused=target.dataset.path;
    for(const row of allRows)if(row.tabIndex!==(row===target?0:-1))row.tabIndex=row===target?0:-1;
    if(hadFocus&&target!==activeRow)target?.focus();
  }
  filter.addEventListener('input',()=>{
    if(!filterFrame)filterFrame=requestAnimationFrame(()=>applyFilter());
  },{signal});
  signal.addEventListener('abort',()=>cancelAnimationFrame(filterFrame),{once:true});
  const tree=node('div',null,'tree'); tree.setAttribute('role','tree'); tree.setAttribute('aria-label','Файлы проекта');
  const browser=node('div',null,'file-browser'), viewer=node('div',null,'file-preview');
  const split=node('div',null,'file-split');split.tabIndex=0;split.setAttribute('role','separator');split.setAttribute('aria-label','Ширина дерева файлов');split.setAttribute('aria-orientation','vertical');
  browser.append(toolbar,tree);root.append(viewer,split,browser);
  root.append(node('style',`:host{flex-direction:row!important;container-type:inline-size}.file-browser{display:flex;flex:0 0 var(--tree-width,42%);min-width:120px;max-width:65%;flex-direction:column}.toolbar{padding:8px;gap:6px}.toolbar input{width:100%;min-width:0;padding:8px;border-radius:10px;border:1px solid var(--border-subtle);background:var(--bg-panel-soft);color:inherit;font:inherit}.row{height:30px;min-height:30px;font:13px var(--font-sans)}.tree{padding:6px}.tree [hidden]{display:none}.file-split{width:5px;flex:none;border-left:1px solid var(--border-subtle);cursor:col-resize;touch-action:none}.file-split:hover,.file-split:focus-visible{background:var(--border-strong)}:host(.tree-hidden) .file-browser,:host(.tree-hidden) .file-split{display:none}`));
  let drag, resizeFrame, resizeWidth;
  function treeWidth(width){const hostWidth=root.host.getBoundingClientRect().width;browser.style.setProperty('--tree-width',`${Math.max(120,Math.min(hostWidth*.65,width))}px`);split.setAttribute('aria-valuenow',String(Math.round(browser.getBoundingClientRect().width)));}
  split.addEventListener('pointerdown',event=>{if(event.button!==0)return;event.preventDefault();drag={id:event.pointerId,x:event.clientX,width:browser.getBoundingClientRect().width};split.setPointerCapture(event.pointerId);},{signal});
  split.addEventListener('pointermove',event=>{
    if(drag?.id!==event.pointerId)return;
    resizeWidth=drag.width+drag.x-event.clientX;
    if(!resizeFrame)resizeFrame=requestAnimationFrame(()=>{resizeFrame=undefined;treeWidth(resizeWidth);});
  },{signal});
  for(const name of ['pointerup','pointercancel','lostpointercapture'])split.addEventListener(name,()=>{
    if(resizeFrame){cancelAnimationFrame(resizeFrame);resizeFrame=undefined;treeWidth(resizeWidth);}
    drag=null;
  },{signal});
  signal.addEventListener('abort',()=>cancelAnimationFrame(resizeFrame),{once:true});
  split.addEventListener('keydown',event=>{if(!['ArrowLeft','ArrowRight'].includes(event.key))return;event.preventDefault();treeWidth(browser.getBoundingClientRect().width+(event.key==='ArrowLeft'?20:-20));},{signal});
  const workspace=services['agent.workspace.read'], expanded=new Set(), listings=new Map(), pending=new Set();
  let generation=0, selected='', focused='', preview, changes=new Map(), gitError='', gitTruncated=false;
  function shape(name,className) {
    const holder=node('span'); icon(holder,name); const svg=holder.firstChild;
    svg.removeAttribute('style'); svg.setAttribute('stroke-linecap','round'); svg.setAttribute('stroke-linejoin','round'); if(className) svg.setAttribute('class',className); svg.setAttribute('aria-hidden','true'); return svg;
  }
  function visibleRows() { return [...tree.querySelectorAll('.row:not(:disabled):not([hidden])')]; }
  function focusPath(path) {
    focused=path;
    for(const row of visibleRows()) { row.tabIndex=row.dataset.path===path?0:-1; if(row.tabIndex===0) row.focus(); }
  }
  const marks={added:'A',modified:'M',deleted:'D',renamed:'R',untracked:'U',conflict:'!'};
  const labels={added:'Добавлен',modified:'Изменён',deleted:'Удалён',renamed:'Переименован',untracked:'Не отслеживается',conflict:'Конфликт'};
  function treeRow(entry,parent,depth,deleted,previous) {
    const folder=entry.kind==='directory', status=changes.get(entry.path), selectedRow=selected===entry.path;
    const row=previous??node('button');
    if(!previous) {
      row.type='button'; row.dataset.path=entry.path; row.setAttribute('role','treeitem');
      row.append(folder?shape('chevron-right','chevron'):node('span',null,'spacer'));
      const type=({rs:'rust',js:'js',jsx:'js',mjs:'js',ts:'js',tsx:'js',json:'json',md:'md',mdx:'md'})[entry.name.split('.').pop().toLowerCase()];
      row.append(shape(folder?'folder':'file',!folder&&type?`file-type-${type}`:undefined),node('span',entry.name,'label'));
    }
    const className=`row ${folder?'folder':'file'}${selectedRow?' active':''}${status?` git-${status}`:''}`;
    if(row.className!==className) row.className=className;
    if(row.dataset.parent!==parent) row.dataset.parent=parent;
    const padding=`${8+depth*14}px`;
    if(row.style.paddingLeft!==padding) row.style.paddingLeft=padding;
    const level=String(depth+1), isSelected=String(selectedRow);
    if(row.getAttribute('aria-level')!==level) row.setAttribute('aria-level',level);
    if(row.getAttribute('aria-selected')!==isSelected) row.setAttribute('aria-selected',isSelected);
    if(folder) {
      const expandedValue=String(expanded.has(entry.path));
      if(row.getAttribute('aria-expanded')!==expandedValue) row.setAttribute('aria-expanded',expandedValue);
    }
    const disabled=!folder&&!deleted&&entry.kind!=='file';
    if(row.disabled!==disabled) row.disabled=disabled;
    const title=`${entry.path}${disabled?' · Просмотр недоступен':''}${status?` · ${labels[status]??status}`:''}`;
    if(row.title!==title) row.title=title;
    const name=row.querySelector('.label');
    if(name.textContent!==entry.name) name.textContent=entry.name;
    let mark=row.querySelector('.git-mark');
    if(status) {
      if(!mark) { mark=node('span',null,'git-mark'); row.append(mark); }
      const label=labels[status]??status;
      if(mark.textContent!==(marks[status]??'')) mark.textContent=marks[status]??'';
      if(mark.getAttribute('aria-label')!==label) mark.setAttribute('aria-label',label);
    } else mark?.remove();
    return row;
  }
  function render() {
    const scroll=tree.scrollTop, activeRow=root.activeElement?.closest?.('.row'), hadFocus=activeRow&&tree.contains(activeRow);
    const previous=new Map([...tree.children].map(child=>[child.dataset.key,child])), wanted=[];
    function keep(key,create) {
      const item=create(previous.get(key));
      if(item.dataset.key!==key) item.dataset.key=key;
      wanted.push(item);
    }
    function status(key,message) {
      keep(`status:${key}`,previous=>{
        const item=previous??node('p',null,'status');
        if(item.textContent!==message) item.textContent=message;
        return item;
      });
    }
    function append(path,depth) {
      const listing=listings.get(path);
      if(!listing) { status(`${path}:loading`,'Загрузка…'); return; }
      if(listing.error) { status(`${path}:error`,listing.error); return; }
      for(const entry of listing.entries) {
        const folder=entry.kind==='directory';
        keep(`entry:${entry.path}:${entry.kind}`,previous=>treeRow(entry,path,depth,false,previous));
        if(folder&&expanded.has(entry.path)) append(entry.path,depth+1);
      }
      if(!listing.entries.length) status(`${path}:empty`,'Пустая папка');
      if(listing.truncated) status(`${path}:truncated`,'Показаны первые 1000 элементов.');
    }
    append('',0);
    const deleted=[...changes].filter(([,status])=>status==='deleted');
    if(deleted.length) status('git:deleted','Удалённые файлы');
    for(const [path] of deleted) keep(`deleted:${path}`,previous=>treeRow({path,name:path,kind:'file'},'',0,true,previous));
    if(gitError) status('git:error',gitError);
    if(gitTruncated) status('git:truncated','Список изменений Git показан не полностью.');
    let cursor=tree.firstChild;
    for(const item of wanted) {
      if(item===cursor) cursor=cursor.nextSibling;
      else tree.insertBefore(item,cursor);
    }
    while(cursor) { const next=cursor.nextSibling; cursor.remove(); cursor=next; }
    if(tree.scrollTop!==scroll) tree.scrollTop=scroll;
    cancelAnimationFrame(filterFrame); applyFilter(activeRow,hadFocus);
  }
  let renderFrame;
  function scheduleRender() {
    if(renderFrame!==undefined) return;
    renderFrame=requestAnimationFrame(()=>{renderFrame=undefined;render();});
  }
  function renderNow() {
    if(renderFrame!==undefined) { cancelAnimationFrame(renderFrame); renderFrame=undefined; }
    render();
  }
  signal.addEventListener('abort',()=>cancelAnimationFrame(renderFrame),{once:true});
  async function loadChanges(revision) {
    try {
      const result=await workspace.changes();
      if(signal.aborted||revision!==generation) return;
      changes=new Map(result.entries.map(entry=>[entry.path,entry.status])); gitError=''; gitTruncated=result.truncated; scheduleRender();
    } catch(error) {
      if(signal.aborted||revision!==generation) return;
      gitError=`Не удалось получить изменения Git: ${error.message}`; scheduleRender();
    }
  }
  async function load(path,revision=generation) {
    if(pending.has(path)||signal.aborted) return;
    pending.add(path);
    try {
      const listing=await workspace.list(path);
      if(signal.aborted||revision!==generation) return;
      listings.set(path,listing); pending.delete(path); scheduleRender();
      for(const entry of listing.entries) if(entry.kind==='directory'&&expanded.has(entry.path)) void load(entry.path,revision);
    } catch(error) {
      if(signal.aborted||revision!==generation) return;
      pending.delete(path); listings.set(path,{error:`Не удалось открыть папку: ${error.message}`}); scheduleRender();
    }
  }
  function toggle(row,open=!expanded.has(row.dataset.path)) {
    const path=row.dataset.path;
    if(open) { expanded.add(path); if(!listings.has(path)||listings.get(path).error) void load(path); }
    else expanded.delete(path);
    focused=path; renderNow();
  }
  function openFile(path) {
    selected=path; focused=path;
    for(const row of visibleRows()) { const active=row.dataset.path===path; row.classList.toggle('active',active); row.setAttribute('aria-selected',String(active)); row.tabIndex=active?0:-1; }
    void preview.open(path,{mode:changes.get(path)==='deleted'?'diff':'file',deleted:changes.get(path)==='deleted'});
  }
  tree.addEventListener('click',event=>{
    const row=event.target.closest('.row'); if(!row||row.disabled) return;
    focused=row.dataset.path;
    if(row.classList.contains('folder')) toggle(row);
    else openFile(row.dataset.path);
  },{signal});
  tree.addEventListener('dblclick',event=>{
    const row=event.target.closest('.file'); if(row&&!row.disabled) openFile(row.dataset.path);
  },{signal});
  tree.addEventListener('focusin',event=>{ if(event.target.matches('.row')) focused=event.target.dataset.path; },{signal});
  tree.addEventListener('keydown',event=>{
    const row=event.target.closest('.row'); if(!row) return;
    const rows=visibleRows(), index=rows.indexOf(row), folder=row.classList.contains('folder');
    let target;
    switch(event.key) {
      case 'Enter': if(folder) toggle(row); else openFile(row.dataset.path); break;
      case 'ArrowDown': target=rows[Math.min(index+1,rows.length-1)]; break;
      case 'ArrowUp': target=rows[Math.max(index-1,0)]; break;
      case 'Home': target=rows[0]; break;
      case 'End': target=rows.at(-1); break;
      case 'ArrowRight': if(folder&&!expanded.has(row.dataset.path)) toggle(row,true); else if(folder&&rows[index+1]?.dataset.parent===row.dataset.path) target=rows[index+1]; break;
      case 'ArrowLeft': if(folder&&expanded.has(row.dataset.path)) toggle(row,false); else target=rows.find(item=>item.dataset.path===row.dataset.parent); break;
      default: return;
    }
    event.preventDefault(); if(target) focusPath(target.dataset.path);
  },{signal});
  function reload() {
    ++generation; pending.clear();
    preview.invalidate();
    // Keep visible rows while fresh directory responses arrive independently.
    for(const path of listings.keys()) if(path&&!expanded.has(path)) listings.delete(path);
    void load(''); void loadChanges(generation);
  }
  preview=createPreview({root:viewer,workspace,signal,onToggleTree:()=>{root.host.classList.toggle('tree-hidden');return !root.host.classList.contains('tree-hidden');}});
  refresh.addEventListener('click',reload,{signal}); renderNow();
  let loaded=false;
  observeVisibility(root, shown=>{
    if(shown&&!loaded){loaded=true;reload();}
  },signal);
}
