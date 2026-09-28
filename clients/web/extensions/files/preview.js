import { node } from '../dom.js';

// Each document uses a host tab, alongside quota, usage and other extensions.
export function createPreview({panels,workspace,signal}) {
  const files=new Map();let nextId=0;
  function open(path,{mode='file',deleted=false}={}) {
    if(signal.aborted)return;
    if(files.has(path)){files.get(path).pane.show();return;}
    const pane=panels.create(`document-${++nextId}`,{title:path,location:'right',onClose:()=>files.delete(path)});
    const root=pane.root, file={pane,mode,views:new Map()};files.set(path,file);
    root.append(node('style',`
      :host{display:flex!important;flex-direction:column;height:100%;min-height:0;overflow:hidden}
      .toolbar{display:flex;align-items:center;gap:8px;flex:none;padding:8px 12px;border-bottom:1px solid var(--border-subtle)}
      .filename{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin:0;font-size:12px;color:var(--text-muted)}
      button{background:transparent;border:0;border-radius:8px;padding:6px 8px;font:12px inherit}
      button[aria-pressed=true]{background:var(--bg-panel-soft)}
      .status{margin:12px;color:var(--text-muted);font-size:12px}.status:empty{display:none}
      pre{flex:1;min-height:0;margin:0;padding:14px;overflow:auto;white-space:pre;tab-size:4;font:12px/1.65 var(--font-mono,monospace)}pre[hidden]{display:none}
      .diff-line{display:block;min-width:max-content;min-height:1.65em}.diff-add{color:#a5d5ad;background:#31583b44}.diff-remove{color:#e5aaaa;background:#70383844}.diff-hunk{color:var(--text-main);background:var(--bg-panel-soft)}.diff-meta{color:var(--text-muted)}
    `));
    const toolbar=node('div',null,'toolbar'),filename=node('p',path,'filename'),code=node('pre'),status=node('p','','status');
    status.setAttribute('role','status');code.tabIndex=0;
    const buttons=new Map();
    for(const [name,label]of [['file','Файл'],['diff','Изменения']]){
      const button=node('button',label);button.type='button';button.dataset.mode=name;button.disabled=name==='file'&&deleted;
      button.addEventListener('click',()=>{save();file.mode=name;void show();},{signal:pane.signal});buttons.set(name,button);toolbar.append(button);
    }
    toolbar.prepend(filename);root.append(toolbar,status,code);
    function save(){const view=file.views.get(file.mode);if(view){view.top=code.scrollTop;view.left=code.scrollLeft;}}
    function render(){
      const view=file.views.get(file.mode);for(const [name,button]of buttons)button.setAttribute('aria-pressed',String(name===file.mode));
      status.textContent=view?.status??'Чтение…';code.hidden=view?.kind!=='text';code.replaceChildren();
      if(view?.kind==='text'&&file.mode==='diff')for(const line of view.text.split('\n')){
        const kind=line.startsWith('+++')||line.startsWith('---')||line.startsWith('diff ')||line.startsWith('index ')?'meta':line.startsWith('+')?'add':line.startsWith('-')?'remove':line.startsWith('@@')?'hunk':'';
        code.append(node('span',line,`diff-line${kind?` diff-${kind}`:''}`));
      }else code.textContent=view?.text??'';
      code.scrollTop=view?.top??0;code.scrollLeft=view?.left??0;
    }
    async function show(){
      const mode=file.mode;
      if(file.views.has(mode)){render();return;}
      const view={kind:'loading',status:'Чтение…',text:'',top:0,left:0};file.views.set(mode,view);render();
      try{
        const result=await (mode==='diff'?workspace.diff(path):workspace.read(path));
        if(signal.aborted||files.get(path)!==file)return;
        view.kind=result.kind;view.text=result.kind==='text'?(mode==='diff'?result.patch:result.text)??'':'';
        view.status=result.kind==='text'?(mode==='diff'&&!view.text?'Нет изменений относительно HEAD.':''):result.kind==='too_large'?'Содержимое слишком большое для предпросмотра.':result.kind==='unavailable'?'Изменения недоступны для этого файла.':'Бинарный файл: текстовый предпросмотр недоступен.';
      }catch(error){if(signal.aborted||files.get(path)!==file)return;view.kind='error';view.status=`Не удалось прочитать файл: ${error.message}`;}
      if(file.mode===mode)render();
    }
    pane.show();void show();
  }
  return {open};
}
