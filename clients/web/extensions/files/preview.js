import { node } from '../dom.js';

// The file viewer and tree share one extension tab. Switching files reuses this surface.
export function createPreview({root,workspace,signal,onToggleTree}) {
  const files=new Map(); let current=null;
  root.append(node('style',`
    .file-preview{display:flex;flex:1;min-width:0;min-height:0;flex-direction:column;overflow:hidden}
    .preview-toolbar{display:flex;align-items:center;gap:6px;flex:none;padding:8px;border-bottom:1px solid var(--border-subtle)}
    .filename{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin:0;font-size:12px;color:var(--text-muted)}
    .preview-toolbar button{background:transparent;border:0;border-radius:8px;padding:5px;min-height:28px;font-size:11px}
    .preview-toolbar button[aria-pressed=true]{background:var(--bg-panel-soft)}
    .preview-status{margin:12px;color:var(--text-muted);font-size:12px;overflow-wrap:anywhere}.preview-status:empty{display:none}
    .file-code{flex:1;min-height:0;margin:0;padding:12px 0;overflow:auto;white-space:pre;tab-size:4;font:12px/1.65 var(--font-mono,monospace);counter-reset:line}
    .file-code[hidden]{display:none}.source-line,.diff-line{display:block;min-width:max-content;min-height:1.65em;padding:0 12px}
    .source-line::before{counter-increment:line;content:counter(line);display:inline-block;width:3em;padding-right:16px;text-align:right;color:var(--text-faint);user-select:none}
    .diff-add{color:#a5d5ad;background:#31583b44}.diff-remove{color:#e5aaaa;background:#70383844}.diff-hunk{color:var(--text-main);background:var(--bg-panel-soft)}.diff-meta{color:var(--text-muted)}
  `));
  const toolbar=node('div',null,'preview-toolbar'), filename=node('p','Открыть файл','filename'), code=node('pre',null,'file-code'),status=node('p','Выберите файл в дереве справа.','preview-status');
  status.setAttribute('role','status');code.tabIndex=0;code.hidden=true;
  const buttons=new Map();
  for(const [name,label]of [['file','Файл'],['diff','Изменения']]){
    const button=node('button',label);button.type='button';button.dataset.mode=name;button.disabled=true;
    button.addEventListener('click',()=>{if(!current)return;save();current.mode=name;void show(current);},{signal});buttons.set(name,button);toolbar.append(button);
  }
  const tree=node('button','Дерево');tree.type='button';tree.title='Показать или скрыть дерево файлов';tree.setAttribute('aria-pressed','true');
  tree.addEventListener('click',()=>tree.setAttribute('aria-pressed',String(onToggleTree())),{signal});
  toolbar.prepend(filename);toolbar.append(tree);root.append(toolbar,status,code);
  function save(){const view=current?.views.get(current.mode);if(view){view.top=code.scrollTop;view.left=code.scrollLeft;}}
  function render(){
    const view=current?.views.get(current.mode);filename.textContent=current?.path.split('/').at(-1)??'Открыть файл';filename.title=current?.path??'';
    for(const [name,button]of buttons){button.setAttribute('aria-pressed',String(name===current?.mode));button.disabled=!current||(name==='file'&&current.deleted);}
    status.textContent=view?.status??'Чтение…';code.hidden=view?.kind!=='text';code.replaceChildren();
    if(view?.kind==='text')for(const line of view.text.split('\n')){
      const kind=line.startsWith('+++')||line.startsWith('---')||line.startsWith('diff ')||line.startsWith('index ')?'meta':line.startsWith('+')?'add':line.startsWith('-')?'remove':line.startsWith('@@')?'hunk':'';
      code.append(node('span',line,current.mode==='diff'?`diff-line${kind?` diff-${kind}`:''}`:'source-line'));
    }
    code.scrollTop=view?.top??0;code.scrollLeft=view?.left??0;
  }
  async function show(file){
    const mode=file.mode;
    if(file.views.has(mode)){render();return;}
    const view={kind:'loading',status:'Чтение…',text:'',top:0,left:0};file.views.set(mode,view);render();
    try{
      const result=await(mode==='diff'?workspace.diff(file.path):workspace.read(file.path));
      if(signal.aborted)return;
      view.kind=result.kind;view.text=result.kind==='text'?(mode==='diff'?result.patch:result.text)??'':'';
      view.status=result.kind==='text'?(mode==='diff'&&!view.text?'Нет изменений относительно HEAD.':''):result.kind==='too_large'?'Содержимое слишком большое для предпросмотра.':result.kind==='unavailable'?'Изменения недоступны для этого файла.':'Бинарный файл: текстовый предпросмотр недоступен.';
    }catch(error){if(signal.aborted)return;view.kind='error';view.status=`Не удалось прочитать файл: ${error.message}`;}
    if(current===file&&file.mode===mode)render();
  }
  function open(path,{mode='file',deleted=false}={}){
    if(signal.aborted)return;save();
    if(!files.has(path))files.set(path,{path,mode,deleted,views:new Map()});
    current=files.get(path);
    // Bound retained text while preserving recently viewed files and scroll positions.
    files.delete(path);files.set(path,current);if(files.size>20)files.delete(files.keys().next().value);
    void show(current);
  }
  return {open};
}
