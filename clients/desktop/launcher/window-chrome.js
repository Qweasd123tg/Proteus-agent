import { popup } from '/ui/popup.js';
// Native chrome belongs to the desktop shell; browser clients keep their own frame.
const api = window.__TAURI__;
if (api) mountWindowChrome(api);

export function mountWindowChrome(api) {
  if (document.querySelector('.desktop-titlebar')) return;
  const win = api.window.getCurrentWindow();
  const controller = new AbortController();
  const options = { signal: controller.signal };
  let disposed = false, releaseResize, revision = 0;
  const style = document.createElement('link');
  style.rel = 'stylesheet'; style.href = '/window-chrome.css';
  document.head.append(style);
  document.documentElement.dataset.desktopChrome = '';
  const bar = document.createElement('header');
  bar.className = 'desktop-titlebar';
  bar.setAttribute('aria-label', 'Окно Proteus');
  bar.innerHTML = `
  <span class="desktop-window-title"></span>
  <span class="desktop-window-error" role="status" hidden></span>
  <div class="desktop-window-controls">
    <button type="button" data-action="minimize" aria-label="Свернуть окно" title="Свернуть окно"><svg viewBox="0 0 16 16"><path d="M3 8h10"/></svg></button>
    <button type="button" data-action="maximize" aria-label="Развернуть окно" title="Развернуть окно"><svg viewBox="0 0 16 16"><path class="maximize-icon" d="M3.5 3.5h9v9h-9z"/><path class="restore-icon" d="M5.5 5.5h7v7h-7z M3.5 10.5v-7h7"/></svg></button>
    <button type="button" data-action="close" aria-label="Закрыть окно" title="Закрыть окно"><svg viewBox="0 0 16 16"><path d="m4 4 8 8m0-8-8 8"/></svg></button>
  </div>`;
  bar.querySelector('.desktop-window-title').textContent = win.label === 'launcher' ? 'Открыть проект' : (window.__PROTEUS_DESKTOP__?.workspace || document.title);
  document.body.prepend(bar);
  const menu = popup('desktop-app-menu-panel','Proteus');menu.element.setAttribute('role','menu');
  const maximize = bar.querySelector('[data-action=maximize]');
  const error = bar.querySelector('.desktop-window-error');
  const report = reason => { error.hidden = false; error.textContent = String(reason); };
  const perform = task => Promise.resolve().then(task).catch(report);
  const refresh = async () => {
    const current = ++revision;
    const [maximized, fullscreen] = await Promise.all([win.isMaximized(), win.isFullscreen()]);
    if (disposed || current !== revision) return;
    bar.classList.toggle('maximized', maximized);
    document.documentElement.classList.toggle('desktop-maximized', maximized || fullscreen);
    document.documentElement.classList.toggle('desktop-fullscreen', fullscreen);
    maximize.title = maximized ? 'Восстановить окно' : 'Развернуть окно';
    maximize.setAttribute('aria-label', maximize.title);
  };
  const actions = {
    project: () => api.core.invoke('open_project'),
    folder: () => api.core.invoke('open_workspace_folder'),
    inspector: () => api.core.invoke('open_client', { label: 'inspector', sessionDir: new URL(location.href).searchParams.get('session_dir') }),
    minimize: () => win.minimize(),
    maximize: async () => { await win.toggleMaximize(); await refresh(); },
    close: () => win.close(),
    quit: () => api.core.invoke('quit_app'),
  };
  bar.addEventListener('click', event => {
    const action = event.target.closest('[data-action]')?.dataset.action;
    if (!actions[action]) return;
    menu.hide();
    perform(actions[action]);
  }, options);
  const dragTarget = event => event.target.closest('.desktop-titlebar, .topbar, .inspector-topbar')
    && !event.target.closest('button, a, input, select, textarea, details');
  document.addEventListener('mousedown', event => {
    if (event.button === 0 && event.detail < 2 && dragTarget(event)) {
      event.preventDefault(); perform(() => win.startDragging());
    }
  }, options);
  document.addEventListener('dblclick', event => {
    if (event.button === 0 && dragTarget(event)) perform(actions.maximize);
  }, options);
  document.addEventListener('click', event => {
    const anchor=event.target.closest('[data-app-menu]');if(!anchor)return;
    menu.show([
      menu.action('Открыть проект…','folder',()=>perform(actions.project)),
      menu.action('Inspector','inspector',()=>perform(actions.inspector)),
      menu.action('Выйти из Proteus','close',()=>perform(actions.quit)),
    ],anchor,{x:anchor.getBoundingClientRect().left,y:anchor.getBoundingClientRect().bottom+6});
  }, options);
  document.addEventListener('proteus-desktop-action',event=>{
    if(['project','folder','inspector'].includes(event.detail))perform(actions[event.detail]);
  },options);
  document.addEventListener('keydown', event => {
    if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.repeat) {
      const action = event.shiftKey ? { KeyO: 'project', KeyI: 'inspector' }[event.code]
        : event.code === 'KeyQ' ? 'quit' : null;
      if (action) {
        event.preventDefault(); event.stopImmediatePropagation(); menu.hide();
        perform(actions[action]); return;
      }
    }
  }, { ...options, capture: true });
  const edges = document.createElement('div');
  edges.className = 'desktop-resize-edges';
  for (const direction of ['North', 'South', 'East', 'West', 'NorthEast', 'NorthWest', 'SouthEast', 'SouthWest']) {
    const edge = document.createElement('div'); edge.dataset.direction = direction;
    edge.addEventListener('mousedown', event => {
      if (event.button !== 0) return;
      event.preventDefault(); perform(() => win.startResizeDragging(direction));
    }, options);
    edges.append(edge);
  }
  document.body.append(edges);
  perform(refresh);
  perform(async () => {
    const release = await win.onResized(() => perform(refresh));
    if (disposed) release(); else releaseResize = release;
  });
  const dispose = () => {
    if (disposed) return;
    disposed = true; controller.abort(); releaseResize?.();
    menu.dispose(); bar.remove(); edges.remove(); style.remove();
    delete document.documentElement.dataset.desktopChrome;
    document.documentElement.classList.remove('desktop-maximized', 'desktop-fullscreen');
  };
  window.addEventListener('pagehide', dispose, { ...options, once: true });
  return dispose;
}
