import { icon } from '../icons.js';
import { node } from '../dom.js';

let nextPreviewId=0;

export function createPreview({ panels, workspace, signal }) {
  const viewId=`files-preview-${++nextPreviewId}`;
  const pane=panels.create('preview',{title:'Просмотр',location:'right'}), root=pane.root;
  root.append(node('style',`
    :host{display:flex!important;flex-direction:column;height:100%;min-height:0;overflow:hidden}
    .tabs{display:flex;flex:none;overflow:auto;border-bottom:1px solid var(--border-subtle,#333);min-height:32px}
    .tab{display:flex;align-items:center;flex:none;border-right:1px solid var(--border-subtle,#333);max-width:230px}.tab.active{background:var(--bg-hover,#333);box-shadow:inset 0 2px var(--accent,#8aaaff)}
    button{border:0;border-radius:0;background:transparent;color:inherit;font-size:12px;cursor:pointer}.tab-name{padding:8px 10px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.tab.transient .tab-name{font-style:italic}.tab-parent{margin-left:7px;font-size:11px;color:var(--text-muted,#aaa)}.close{display:grid;place-items:center;padding:5px 7px}.close svg{width:14px;height:14px}.close:hover{background:var(--bg-hover,#444)}
    .filename{flex:1;min-width:0;margin:0;padding:7px 12px;color:var(--text-muted,#aaa);font:11px var(--font-mono,monospace);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .toolbar{display:flex;align-items:center;flex-wrap:wrap;flex:none;border-bottom:1px solid var(--border-subtle,#333)}.modes{display:flex;flex:none;padding:2px 6px}.modes button{padding:5px 7px}.modes [aria-pressed=true]{background:var(--bg-hover,#333);border-radius:4px}.modes button:disabled{opacity:.4;cursor:default}
    .status{margin:8px 12px;font-size:12px;overflow-wrap:anywhere}.status:empty{display:none}
    pre{flex:1;min-height:0;min-width:0;margin:0;padding:8px 12px;overflow:auto;tab-size:4;font:12px/1.6 var(--font-mono,monospace);white-space:pre}pre[hidden]{display:none}
    .diff-line{display:block;min-width:max-content;min-height:1.6em}.diff-add{color:#a5d5ad;background:#31583b44}.diff-remove{color:#e5aaaa;background:#70383844}.diff-hunk{color:#9abfdf;background:#344d6344}.diff-meta{color:var(--text-muted,#aaa)}
  `));
  const tabs=node('div',null,'tabs'); tabs.setAttribute('role','tablist'); tabs.setAttribute('aria-label','Открытые файлы');
  const toolbar=node('div',null,'toolbar'), filename=node('p','','filename'), modes=node('div',null,'modes');
  const fileButton=node('button','Файл'), diffButton=node('button','Изменения'); fileButton.type=diffButton.type='button';
  fileButton.dataset.mode='file'; diffButton.dataset.mode='diff'; modes.append(fileButton,diffButton); toolbar.append(filename,modes);
  const status=node('p','','status'), code=node('pre'); status.setAttribute('role','status'); code.tabIndex=0; code.setAttribute('role','tabpanel');code.id=`${viewId}-content`;
  root.append(tabs,toolbar,status,code);
  const files=new Map(); let active='', nextId=0, tabSignature='';
  const fresh=()=>({kind:'loading',status:'Чтение…',text:'',top:0,left:0,started:false});
  function current(file=files.get(active)) {return file?.[file.mode];}
  function saveScroll() { const view=current(); if(view) {view.top=code.scrollTop;view.left=code.scrollLeft;} }
  function pin(path) {const file=files.get(path);if(file){file.pinned=true;render();tabs.querySelector('[aria-selected=true]')?.focus();}}
  function parentLabel(path) {
    const parts=path.split('/'), base=parts.pop(), peers=[...files.keys()].filter(other=>other!==path&&other.split('/').pop()===base);
    if(!peers.length)return '';
    for(let depth=1;depth<=parts.length;depth++) {
      const suffix=parts.slice(-depth).join('/');
      if(peers.every(other=>other.split('/').slice(0,-1).slice(-depth).join('/')!==suffix))return suffix;
    }
    return parts.join('/')||'.';
  }
  function render() {
    const signature=JSON.stringify([...files].map(([path,file])=>[path,file.id,file.pinned]));
    if(signature!==tabSignature) {
      tabSignature=signature; const fragment=document.createDocumentFragment();
      for(const [path,file] of files) {
        const tab=node('div',null,`tab${file.pinned?'':' transient'}`), name=node('button',path.split('/').pop(),'tab-name'), close=node('button',null,'close');
        tab.dataset.path=path; name.type=close.type='button'; name.title=path; name.id=`${viewId}-tab-${file.id}`; name.setAttribute('role','tab');name.setAttribute('aria-controls',code.id);
        const parent=parentLabel(path);if(parent)name.append(node('span',parent,'tab-parent'));
        name.addEventListener('click',()=>select(path)); name.addEventListener('dblclick',()=>pin(path));
        name.addEventListener('keydown',event=>{
          const paths=[...files.keys()], index=paths.indexOf(path); let target;
          if(event.key==='ArrowRight') target=paths[(index+1)%paths.length];
          else if(event.key==='ArrowLeft') target=paths[(index+paths.length-1)%paths.length];
          else if(event.key==='Home') target=paths[0]; else if(event.key==='End') target=paths.at(-1);
          else if(event.key==='Enter') {event.preventDefault();pin(path);return;}
          else if(event.key==='Delete') {event.preventDefault();remove(path);tabs.querySelector('[aria-selected=true]')?.focus();return;} else return;
          event.preventDefault();select(target);tabs.querySelector('[aria-selected=true]')?.focus();
        });
        close.append(icon('close')); close.title=`Закрыть ${path}`;close.setAttribute('aria-label',close.title);close.addEventListener('click',()=>remove(path));
        tab.append(name,close);fragment.append(tab);
      }
      tabs.replaceChildren(fragment);
    }
    for(const tab of tabs.children) {
      const selected=tab.dataset.path===active;tab.classList.toggle('active',selected);
      const name=tab.querySelector('[role=tab]');name.setAttribute('aria-selected',String(selected));name.tabIndex=selected?0:-1;
    }
    const file=files.get(active), view=current(file); filename.textContent=active;filename.title=active;
    fileButton.disabled=!file||file.deleted;diffButton.disabled=!file;
    fileButton.setAttribute('aria-pressed',String(file?.mode==='file'));diffButton.setAttribute('aria-pressed',String(file?.mode==='diff'));
    code.replaceChildren(); code.hidden=view?.kind!=='text';status.textContent=view?.status??'';
    if(view?.kind==='text'&&file.mode==='diff') {
      const fragment=document.createDocumentFragment();
      for(const line of view.text.split('\n')) {
        const kind=line.startsWith('+++')||line.startsWith('---')||line.startsWith('diff ')||line.startsWith('index ')?'meta':line.startsWith('+')?'add':line.startsWith('-')?'remove':line.startsWith('@@')?'hunk':'';
        fragment.append(node('span',line,`diff-line${kind?` diff-${kind}`:''}`));
      }
      code.append(fragment);
    } else code.textContent=view?.text??'';
    if(file) code.setAttribute('aria-labelledby',`${viewId}-tab-${file.id}`);else code.removeAttribute('aria-labelledby');
    code.scrollTop=view?.top??0;code.scrollLeft=view?.left??0;
  }
  function select(path) {saveScroll();active=path;render();pane.show();}
  function remove(path) {
    saveScroll();const paths=[...files.keys()],index=paths.indexOf(path);files.delete(path);
    if(active===path) active=paths[index+1]??paths[index-1]??'';
    render();if(!files.size)pane.hide();
  }
  async function read(path,file,mode) {
    const view=file[mode];if(view.started)return;view.started=true;
    try {
      const result=await (mode==='diff'?workspace.diff(path):workspace.read(path));
      if(signal.aborted||files.get(path)!==file)return;
      view.kind=result.kind;view.text=result.kind==='text'?(mode==='diff'?result.patch:result.text)??'':'';
      view.status=result.kind==='text'?(mode==='diff'&&!view.text?'Нет изменений относительно HEAD.':''):result.kind==='too_large'?'Содержимое слишком большое для предпросмотра.':result.kind==='unavailable'?'Изменения недоступны для этого файла.':'Бинарный файл: текстовый предпросмотр недоступен.';
    } catch(error) {
      if(signal.aborted||files.get(path)!==file)return;
      view.kind='error';view.status=`Не удалось ${mode==='diff'?'получить изменения':'прочитать файл'}: ${error.message}`;
    }
    if(active===path&&file.mode===mode){saveScroll();render();}
  }
  function changeMode(mode) {
    const file=files.get(active);if(!file||(mode==='file'&&file.deleted))return;
    saveScroll();file.mode=mode;render();void read(active,file,mode);
  }
  fileButton.addEventListener('click',()=>changeMode('file'),{signal});diffButton.addEventListener('click',()=>changeMode('diff'),{signal});
  function open(path,{pinned=false,mode='file',deleted=false}={}) {
    if(signal.aborted)return;
    if(files.has(path)){const file=files.get(path);if(pinned)file.pinned=true;select(path);return;}
    saveScroll();if(!pinned)for(const [previous,file]of files)if(!file.pinned)files.delete(previous);
    const file={id:++nextId,pinned,mode,deleted,file:fresh(),diff:fresh()};files.set(path,file);select(path);void read(path,file,mode);
  }
  return {open};
}
