import { node } from '../dom.js';

export function mount({ root, compact, panel, services, signal }) {
  compact.innerHTML = '<svg viewBox="0 0 24 24" width="25" height="25" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3 7V5h6l2 2h10v13H3Z"/></svg>';
  const style = node('style', `
    .toolbar{display:flex;gap:8px;align-items:center;margin-bottom:10px}.toolbar button{font-size:12px}
    .layout{display:grid;gap:12px;min-width:0}.tree{overflow:auto;max-height:55vh;min-width:0}
    .branch{padding-left:13px;border-left:1px solid var(--border-subtle,#333);margin-left:5px}
    summary{cursor:pointer;list-style:none;padding:5px 0;font-size:12px;overflow-wrap:anywhere}summary::-webkit-details-marker{display:none}
    summary::before{content:'+';display:inline-block;width:16px;color:var(--text-muted,#999)}details[open]>summary::before{content:'−'}
    .file{display:block;text-align:left;width:100%;padding:5px 6px;font-size:12px;border:0;border-radius:5px;white-space:normal;overflow-wrap:anywhere}
    .file.active{background:var(--bg-hover,#333)}.file:disabled{opacity:.45}
    .preview{min-width:0}.filename{font:12px var(--font-mono,monospace);overflow-wrap:anywhere;margin:0 0 8px}
    pre{margin:0;padding:12px;background:var(--bg-panel-soft,#222);border-radius:8px;overflow:auto;max-height:65vh;tab-size:4;font:12px/1.6 var(--font-mono,monospace);white-space:pre}
    .status{font-size:12px;overflow-wrap:anywhere}.status:empty{display:none}
    @container(min-width:600px){.layout{grid-template-columns:minmax(180px,25%) minmax(0,1fr)}.tree{max-height:70vh}}
  `);
  const toolbar = node('div', null, 'toolbar'), refresh = node('button','Обновить'), center = node('button','В центре');
  refresh.type = center.type = 'button'; toolbar.append(refresh,center);
  center.addEventListener('click', () => panel.move('main'), {signal});
  const tree = node('div', null, 'tree'); tree.setAttribute('aria-label','Файлы проекта');
  const preview = node('section', null, 'preview'), filename = node('h3', 'Выберите файл', 'filename');
  const status = node('p', '', 'status'); status.setAttribute('role','status');
  const code = node('pre'); code.tabIndex=0; code.hidden=true;
  preview.append(filename,status,code);
  const layout = node('div', null, 'layout'); layout.append(tree,preview); root.append(style,toolbar,layout);
  let generation=0, selection=0, activeButton;
  const workspace = services['agent.workspace.read'];
  async function open(path, button) {
    const current = ++selection; status.textContent='Чтение…';
    try {
      const file = await workspace.read(path);
      if (signal.aborted || current !== selection) return;
      activeButton?.classList.remove('active'); button.classList.add('active'); activeButton=button;
      filename.textContent=file.path;
      status.textContent=file.kind === 'text' ? '' : file.kind === 'too_large' ? 'Файл больше 512 КиБ. Предпросмотр недоступен.' : 'Бинарный файл: текстовый предпросмотр недоступен.';
      code.hidden=file.kind !== 'text'; code.textContent=file.text ?? ''; code.scrollTop=0; code.scrollLeft=0;
    } catch(error) { if (!signal.aborted && current === selection) status.textContent=`Не удалось прочитать файл: ${error.message}`; }
  }
  async function directory(path, parent, revision) {
    const loading = node('p','Загрузка…','muted'); parent.append(loading);
    try {
      const listing = await workspace.list(path);
      if (signal.aborted || generation !== revision) return;
      const fragment = document.createDocumentFragment();
      for (const entry of listing.entries) {
        if (entry.kind === 'directory') {
          const branch = node('details'), label = node('summary',entry.name), children = node('div',null,'branch'); let loaded=false;
          label.title=entry.path; branch.append(label,children);
          branch.addEventListener('toggle', () => {
            if (branch.open && !loaded) { loaded=true; void directory(entry.path, children, revision); }
          }, {signal}); fragment.append(branch);
        } else {
          const button = node('button',entry.name,'file'); button.type='button'; button.title=entry.path;
          button.disabled=entry.kind !== 'file';
          if(button.disabled) button.title+=' · Ссылки и специальные файлы не открываются из дерева';
          button.addEventListener('click',()=>void open(entry.path,button),{signal}); fragment.append(button);
        }
      }
      if (!listing.entries.length) fragment.append(node('p','Пустая папка','muted'));
      if (listing.truncated) fragment.append(node('p','Показаны первые 1000 элементов.','muted'));
      parent.replaceChildren(fragment);
    } catch(error) { if (!signal.aborted && generation === revision) loading.textContent=`Не удалось открыть папку: ${error.message}`; }
  }
  function reload() { ++generation; tree.replaceChildren(); void directory('',tree,generation); }
  refresh.addEventListener('click',reload,{signal}); reload();
}
