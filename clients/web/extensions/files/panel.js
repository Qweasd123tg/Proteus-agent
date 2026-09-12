import { node, icon } from '../dom.js';
import { createPreview } from './preview.js';

export function mount({ root, compact, panels, services, signal }) {
  icon(compact, 'M3 7V5h6l2 2h10v13H3Z');
  root.append(node('style', `
    :host{display:flex!important;flex-direction:column;height:100%;min-height:0;overflow:hidden}
    .toolbar{display:flex;align-items:center;justify-content:flex-end;flex:none;padding:4px 8px;border-bottom:1px solid var(--border-subtle,#333);font-size:11px;color:var(--text-muted,#aaa)}
    .toolbar button{display:flex;align-items:center;padding:3px 6px;font-size:12px}.toolbar svg{width:16px;height:16px}.tree{flex:1;min-height:0;overflow:auto;padding:4px 0;outline:none}
    .row{display:flex;align-items:center;justify-content:flex-start;gap:5px;width:100%;height:25px;min-height:25px;box-sizing:border-box;border:0;border-radius:0;padding:0 8px;text-align:left;background:transparent;color:inherit;font:12px var(--font-mono,monospace);white-space:nowrap;cursor:pointer}
    .row:hover{background:var(--bg-hover,#333)}.row.active{background:var(--bg-active,var(--bg-hover,#333))}.row:focus-visible{outline:1px solid var(--accent,#8aaaff);outline-offset:-1px}.row:disabled{opacity:.45;cursor:default}
    .row svg{width:14px;height:14px;flex:none}.row .chevron{width:10px;height:10px;transition:transform .1s}.row[aria-expanded=true] .chevron{transform:rotate(90deg)}
    @media(prefers-reduced-motion:reduce){.row .chevron{transition:none}}
    .file-type-rust{color:#d9a18b}.file-type-js{color:#d8c979}.file-type-json{color:#c7bd84}.file-type-md{color:#86b4d4}
    .git-added .label,.git-untracked .label{color:#8fc99b}.git-modified .label,.git-renamed .label{color:#d8bc79}.git-deleted .label,.git-conflict .label{color:#df9292}.git-deleted .label{text-decoration:line-through}.git-mark{margin-left:auto;flex:none;font-size:10px;color:var(--text-muted,#aaa)}
    .label{overflow:hidden;text-overflow:ellipsis}.spacer{width:10px;flex:none}.status{margin:6px 10px;font-size:12px;overflow-wrap:anywhere;color:var(--text-muted,#aaa)}
  `));
  const toolbar=node('div',null,'toolbar'), refresh=node('button');
  refresh.append(shape('M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.6-1L20 9M4 15l2.3 3A7 7 0 0 0 17.9 17'));
  refresh.type='button'; refresh.title='Обновить дерево'; refresh.setAttribute('aria-label','Обновить дерево');
  toolbar.append(refresh);
  const tree=node('div',null,'tree'); tree.setAttribute('role','tree'); tree.setAttribute('aria-label','Файлы проекта');
  root.append(toolbar,tree);
  const workspace=services['agent.workspace.read'], expanded=new Set(), listings=new Map(), pending=new Set();
  let generation=0, selected='', focused='', preview, changes=new Map(), gitError='', gitTruncated=false;
  function shape(path,className) {
    const holder=node('span'); icon(holder,path); const svg=holder.firstChild;
    svg.removeAttribute('style'); svg.setAttribute('stroke-linecap','round'); svg.setAttribute('stroke-linejoin','round'); if(className) svg.setAttribute('class',className); svg.setAttribute('aria-hidden','true'); return svg;
  }
  function visibleRows() { return [...tree.querySelectorAll('.row:not(:disabled)')]; }
  function focusPath(path) {
    focused=path;
    for(const row of visibleRows()) { row.tabIndex=row.dataset.path===path?0:-1; if(row.tabIndex===0) row.focus(); }
  }
  function render() {
    const scroll=tree.scrollTop, hadFocus=root.activeElement && tree.contains(root.activeElement), fragment=document.createDocumentFragment();
    function append(path,depth) {
      const listing=listings.get(path);
      if(!listing) { fragment.append(node('p','Загрузка…','status')); return; }
      if(listing.error) { fragment.append(node('p',listing.error,'status')); return; }
      for(const entry of listing.entries) {
        const folder=entry.kind==='directory', row=node('button',null,`row ${folder?'folder':'file'}${selected===entry.path?' active':''}`);
        row.type='button'; row.dataset.path=entry.path; row.dataset.parent=path; row.title=entry.path;
        row.style.paddingLeft=`${8+depth*14}px`; row.setAttribute('role','treeitem'); row.setAttribute('aria-level',String(depth+1));
        row.setAttribute('aria-selected',String(selected===entry.path)); row.tabIndex=focused===entry.path?0:-1;
        if(folder) { row.setAttribute('aria-expanded',String(expanded.has(entry.path))); row.append(shape('m9 5 7 7-7 7','chevron')); }
        else row.append(node('span',null,'spacer'));
        const type=({rs:'rust',js:'js',jsx:'js',mjs:'js',ts:'js',tsx:'js',json:'json',md:'md',mdx:'md'})[entry.name.split('.').pop().toLowerCase()];
        row.append(shape(folder?'M3 7V5h6l2 2h10v13H3Z':'M5 3h9l5 5v13H5ZM14 3v6h5',!folder&&type?`file-type-${type}`:undefined),node('span',entry.name,'label'));
        if(!folder) decorate(row,entry.path);
        row.disabled=!folder&&entry.kind!=='file';
        if(row.disabled) row.title+=' · Просмотр недоступен';
        fragment.append(row);
        if(folder&&expanded.has(entry.path)) append(entry.path,depth+1);
      }
      if(!listing.entries.length) fragment.append(node('p','Пустая папка','status'));
      if(listing.truncated) fragment.append(node('p','Показаны первые 1000 элементов.','status'));
    }
    append('',0);
    const deleted=[...changes].filter(([,status])=>status==='deleted');
    if(deleted.length) fragment.append(node('p','Удалённые файлы','status'));
    for(const [path] of deleted) {
      const row=node('button',null,`row file${selected===path?' active':''}`); row.type='button'; row.dataset.path=path; row.dataset.parent=''; row.title=path;
      row.setAttribute('role','treeitem'); row.setAttribute('aria-level','1'); row.setAttribute('aria-selected',String(selected===path)); row.tabIndex=focused===path?0:-1;
      row.append(node('span',null,'spacer'),shape('M5 3h9l5 5v13H5ZM14 3v6h5'),node('span',path,'label')); decorate(row,path); fragment.append(row);
    }
    if(gitError) fragment.append(node('p',gitError,'status'));
    if(gitTruncated) fragment.append(node('p','Список изменений Git показан не полностью.','status'));
    tree.replaceChildren(fragment); tree.scrollTop=scroll;
    const rows=visibleRows();
    if(!rows.some(row=>row.tabIndex===0)&&rows.length) { rows[0].tabIndex=0; focused=rows[0].dataset.path; }
    if(hadFocus) focusPath(focused);
  }
  function decorate(row,path) {
    const status=changes.get(path); if(!status) return;
    const marks={added:'A',modified:'M',deleted:'D',renamed:'R',untracked:'U',conflict:'!'};
    const labels={added:'Добавлен',modified:'Изменён',deleted:'Удалён',renamed:'Переименован',untracked:'Не отслеживается',conflict:'Конфликт'};
    row.classList.add(`git-${status}`); row.title+=` · ${labels[status]??status}`;
    const mark=node('span',marks[status]??'', 'git-mark'); mark.setAttribute('aria-label',labels[status]??status); row.append(mark);
  }
  async function loadChanges(revision) {
    try {
      const result=await workspace.changes();
      if(signal.aborted||revision!==generation) return;
      changes=new Map(result.entries.map(entry=>[entry.path,entry.status])); gitError=''; gitTruncated=result.truncated; render();
    } catch(error) {
      if(signal.aborted||revision!==generation) return;
      gitError=`Не удалось получить изменения Git: ${error.message}`; render();
    }
  }
  async function load(path,revision=generation) {
    if(pending.has(path)||signal.aborted) return;
    pending.add(path);
    try {
      const listing=await workspace.list(path);
      if(signal.aborted||revision!==generation) return;
      listings.set(path,listing); pending.delete(path); render();
      for(const entry of listing.entries) if(entry.kind==='directory'&&expanded.has(entry.path)) void load(entry.path,revision);
    } catch(error) {
      if(signal.aborted||revision!==generation) return;
      pending.delete(path); listings.set(path,{error:`Не удалось открыть папку: ${error.message}`}); render();
    }
  }
  function toggle(row,open=!expanded.has(row.dataset.path)) {
    const path=row.dataset.path;
    if(open) { expanded.add(path); if(!listings.has(path)||listings.get(path).error) void load(path); }
    else expanded.delete(path);
    focused=path; render();
  }
  function openFile(path,pinned=false) {
    selected=path; focused=path;
    for(const row of visibleRows()) { const active=row.dataset.path===path; row.classList.toggle('active',active); row.setAttribute('aria-selected',String(active)); row.tabIndex=active?0:-1; }
    preview??=createPreview({panels,workspace,signal}); void preview.open(path,{pinned,mode:changes.get(path)==='deleted'?'diff':'file',deleted:changes.get(path)==='deleted'});
  }
  tree.addEventListener('click',event=>{
    const row=event.target.closest('.row'); if(!row||row.disabled) return;
    focused=row.dataset.path;
    if(row.classList.contains('folder')) toggle(row);
    else openFile(row.dataset.path);
  },{signal});
  tree.addEventListener('dblclick',event=>{
    const row=event.target.closest('.file'); if(row&&!row.disabled) openFile(row.dataset.path,true);
  },{signal});
  tree.addEventListener('focusin',event=>{ if(event.target.matches('.row')) focused=event.target.dataset.path; },{signal});
  tree.addEventListener('keydown',event=>{
    const row=event.target.closest('.row'); if(!row) return;
    const rows=visibleRows(), index=rows.indexOf(row), folder=row.classList.contains('folder');
    let target;
    switch(event.key) {
      case 'Enter': if(folder) toggle(row); else openFile(row.dataset.path,true); break;
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
    // Keep visible rows while fresh directory responses arrive independently.
    for(const path of listings.keys()) if(path&&!expanded.has(path)) listings.delete(path);
    void load(''); void loadChanges(generation);
  }
  refresh.addEventListener('click',reload,{signal}); render(); reload();
}
