"""Desktop titlebar over the real client; native window calls have an explicit test adapter."""
import base64
import json
from pathlib import Path
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin):
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return !!document.querySelector('.topbar') && document.querySelector('.connection-badge')?.classList.contains('completed')"), 'Chat did not load for native chrome checks')
    fixture = """
      window.chromeCalls=[];window.chromeMax=false;window.chromeFull=false;window.chromeReleased=0;
      const native={label:'main',isMaximized:async()=>chromeMax,isFullscreen:async()=>chromeFull,
        toggleMaximize:async()=>{chromeMax=!chromeMax;chromeCalls.push('maximize')},
        minimize:async()=>chromeCalls.push('minimize'),close:async()=>chromeCalls.push('close'),
        startDragging:async()=>chromeCalls.push('drag'),startResizeDragging:async d=>chromeCalls.push(d),
        onResized:async f=>{window.chromeResize=f;return ()=>window.chromeReleased++}};
      const {mountWindowChrome}=await import('/window-chrome.js');
      window.disposeChrome=mountWindowChrome({window:{getCurrentWindow:()=>native},core:{invoke:async(name)=>chromeCalls.push(name)}});
    """
    # Load in the page realm: WebDriver imports have an independent module map.
    js("const script=document.createElement('script');script.type='module';script.textContent="+json.dumps(fixture)+";document.head.append(script)")
    wait_for(lambda: js("return !!document.querySelector('.desktop-titlebar')"), 'Native chrome did not mount')
    wait_for(lambda: js("return getComputedStyle(document.querySelector('.topbar')).position==='fixed'"), 'Native titlebar stylesheet did not load')
    for width in (1440, 860):
        command('/window/rect', {'width': width, 'height': 1000})
        wait_for(lambda: js("const h=document.querySelector('.topbar').getBoundingClientRect(), b=document.querySelector('.desktop-titlebar').getBoundingClientRect(), controls=document.querySelector('.desktop-window-controls').getBoundingClientRect(), app=document.querySelector('.app-layout').getBoundingClientRect();return h.top===0 && h.height===b.height && h.right<=controls.left && h.left>=44 && app.top===b.bottom && app.bottom<=innerHeight+1 && document.documentElement.scrollWidth<=innerWidth"), 'Window controls and chat header did not share one row')
    assert js("const b=document.querySelector('.topbar [data-panel-toggle=sidebar]').getBoundingClientRect();return b.top>=0&&b.bottom<=40&&b.right<=44"), 'Sidebar toggle did not join the native header'
    js("document.querySelector('.sidebar-search input').focus();document.querySelector('[data-panel-toggle=sidebar]').click()")
    wait_for(lambda: js("return document.activeElement.matches('.topbar [data-panel-toggle=sidebar]')"), 'Sidebar focus did not move to header')
    js("document.querySelector('[data-panel-toggle=sidebar]').click()")
    js("document.querySelector('[data-action=minimize]').click();document.querySelector('[data-action=maximize]').click()")
    wait_for(lambda: js("return chromeCalls.includes('minimize') && document.querySelector('[data-action=maximize]').dataset.uiTooltip==='Восстановить окно'"), 'Native minimize/maximize action failed')
    assert js("return getComputedStyle(document.querySelector('.desktop-resize-edges')).display==='none'"), 'Maximized window kept resize edges'
    js("document.querySelector('.topbar').dispatchEvent(new MouseEvent('dblclick',{bubbles:true,button:0,detail:2}))")
    wait_for(lambda: js("return document.querySelector('[data-action=maximize]').dataset.uiTooltip==='Развернуть окно'"), 'Double click did not restore the window')
    js("document.querySelector('.topbar').dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,detail:1}));document.querySelector('[data-direction=SouthEast]').dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0}))")
    wait_for(lambda: js("return chromeCalls.includes('drag') && chromeCalls.includes('SouthEast')"), 'Drag/resize did not reach the native API')
    js("document.querySelector('.sidebar-surface [data-app-menu]').click()")
    Path('/tmp/proteus-window-chrome.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    assert js("return !document.querySelector('.desktop-app-menu-panel').matches(':popover-open') && document.activeElement.matches('[data-app-menu]')"), 'Menu did not return focus on Escape'
    assert js("return !document.querySelector('.desktop-titlebar [data-app-menu], .desktop-titlebar details')"), 'Duplicate Proteus menu remains'
    for index, native in [(0,'open_project'), (2,'quit_app')]:
        js(f"document.querySelector('.sidebar-surface [data-app-menu]').click();document.querySelectorAll('.desktop-app-menu-panel button')[{index}].click()")
        wait_for(lambda: js(f"return chromeCalls.includes('{native}')"), f'Missing native {native} action')
    js("document.querySelector('[data-action=close]').click();document.dispatchEvent(new CustomEvent('proteus-desktop-action',{detail:'folder'}))")
    wait_for(lambda: js("return chromeCalls.includes('close') && chromeCalls.includes('open_workspace_folder')"), 'Native close/folder action failed')
    js("window.chromeCalls.length=0;for(const code of ['KeyO','KeyI','KeyQ']) document.dispatchEvent(new KeyboardEvent('keydown',{code,ctrlKey:true,shiftKey:code!=='KeyQ',bubbles:true}))")
    wait_for(lambda: js("return ['open_project','quit_app'].every(name=>chromeCalls.includes(name)) && !chromeCalls.includes('open_client') && !!document.querySelector('.settings-page')"), 'Desktop shortcuts failed')
    # Native shortcuts use the same editor and must not fire during capture.
    command('/window/rect', {'width':1440, 'height':1000})
    js("document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=shortcuts]')"), 'Settings navigation pending')
    js("document.querySelector('[data-settings-section=shortcuts]').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-bind=project]')"), 'Native shortcut editor missing')
    js("window.chromeCalls.length=0;document.querySelector('[data-bind=project]').click();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyQ',key:'q',ctrlKey:true,bubbles:true,cancelable:true}))")
    assert js("return chromeCalls.length===0 && document.querySelector('.shortcut-settings [role=status]').textContent.includes('Уже назначено')"), 'Capture executed native quit or accepted a conflict'
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyO',key:'o',ctrlKey:true,altKey:true,bubbles:true,cancelable:true}))")
    js("document.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyO',ctrlKey:true,shiftKey:true,bubbles:true}))")
    assert js("return chromeCalls.length===0"), 'Old native shortcut remains active'
    js("document.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyO',ctrlKey:true,altKey:true,bubbles:true}))")
    wait_for(lambda: js("return chromeCalls.filter(c=>c==='open_project').length===1"), 'Remapped native shortcut failed or dispatched twice')
    js("document.querySelector('[data-reset-shortcuts]').click();document.querySelector('.settings-back').click()")
    js('window.chromeFull=true;window.chromeResize()')
    wait_for(lambda: js("return getComputedStyle(document.querySelector('.desktop-titlebar')).display==='none'"), 'Fullscreen retained the titlebar')
    js('window.disposeChrome();window.disposeChrome()')
    disposed = js("return {released:window.chromeReleased,bar:!!document.querySelector('.desktop-titlebar'),attribute:document.documentElement.hasAttribute('data-desktop-chrome')}")
    assert disposed == {'released': 1, 'bar': False, 'attribute': False}, f'Chrome cleanup failed: {disposed}'
    print('PASS: unified titlebar geometry; window controls; drag/resize; menu focus; fullscreen; disposal', flush=True)
