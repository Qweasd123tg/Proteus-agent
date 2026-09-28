"""File content and tree share a tab; other extensions keep their live roots."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    def shadow(id):
        return f"document.querySelector('[data-extension-id=\"{id}\"] .extension-panel-content')?.shadowRoot"

    def choose(id):
        js("if(document.querySelector('.tab-workspace').hidden)document.querySelector('[data-workspace-toggle]').click();document.querySelector('.workspace-add').click()")
        wait_for(lambda: js("return document.querySelector('.workspace-picker').matches(':popover-open')"), 'Tab picker did not open')
        js(f"document.querySelector('.workspace-picker [data-open-tab=\"{id}\"]').click()")
        wait_for(lambda: js(f"return document.querySelector('.workspace-tab.active').dataset.tabId==='{id}'"), 'Tab did not activate: '+id)

    def active():
        return "document.querySelector('.workspace-tab-content > .extension-panel:not([hidden]) .extension-panel-content').shadowRoot"

    command('/window/rect', {'width': 1440, 'height': 1000})
    assert js("return document.querySelectorAll('.topbar [data-panel-toggle=sidebar]').length===1 && !document.querySelector('.sidebar [data-panel-toggle]')"), 'Sidebar toggle is duplicated or outside header'
    js("document.querySelector('.sidebar-search input').focus();document.querySelector('[data-panel-toggle=sidebar]').click()")
    wait_for(lambda: js("return document.querySelector('.app-layout').classList.contains('sidebar-collapsed') && document.activeElement.matches('.topbar [data-panel-toggle=sidebar]')"), 'Header sidebar toggle/focus failed')
    js("document.querySelector('[data-panel-toggle=sidebar]').click()")
    assert js("const rgb=getComputedStyle(document.body).backgroundColor.match(/\\d+/g);return rgb[0]===rgb[1]&&rgb[1]===rgb[2]"), 'Main palette is not neutral gray'
    choose('model-quota')
    wait_for(lambda: js(f"return {shadow('model-quota')}?.textContent.includes('73% осталось')"), 'Quota data missing')
    js("window.keptQuota=document.querySelector('[data-extension-id=model-quota]');window.keptChat=document.querySelector('.session-workspace')")
    choose('usage')
    choose('model-quota')
    assert js("return document.querySelector('[data-extension-id=model-quota]')===window.keptQuota"), 'Switching tabs remounted quota'
    choose('files')
    files=shadow('files')
    wait_for(lambda: js(f"return !!{files}?.querySelector('.file')"), 'File tree did not load')
    js(f"[...{files}.querySelectorAll('.folder')].find(row=>row.querySelector('.label').textContent==='preview-fixture').click()")
    wait_for(lambda: js(f"return [...{files}.querySelectorAll('.file')].some(row=>row.querySelector('.label').textContent==='hello world.txt')"), 'Directory did not expand')
    js(f"[...{files}.querySelectorAll('.file')].find(row=>row.querySelector('.label').textContent==='hello world.txt').click()")
    wait_for(lambda: js(f"return {active()}?.querySelector('pre')?.textContent.includes('<b>Привет</b>')"), 'Document did not open')
    assert js(f"return !{active()}.querySelector('pre b')"), 'Document interpreted HTML'
    assert js("return document.querySelectorAll('.tab-workspace').length===1 && !document.querySelector('.extension-column')"), 'Documents added another column'
    assert js("return document.querySelector('.workspace-tab.active').dataset.tabId==='files' && !document.querySelector('[data-tab-id^=\"files:document-\"]')"), 'File escaped its browser tab'
    assert js(f"const r={files};return r.querySelector('.file-browser').getBoundingClientRect().width>100 && r.querySelector('.file-preview').getBoundingClientRect().width>100"), 'Tree or preview is symbolic'
    js(f"window.docRoot={active()};window.docTab=document.querySelector('.workspace-tab.active').dataset.tabId;{active()}.querySelector('[data-mode=diff]').click()")
    wait_for(lambda: js(f"return [...{active()}.querySelectorAll('.diff-add')].some(line=>line.textContent.includes('<b>Привет</b>'))"), 'File diff missing')
    js(f"{active()}.querySelector('[data-mode=file]').click()")
    assert js(f"return {active()}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'File view did not restore'
    choose('files')
    wait_for(lambda: js(f"return !!{files}.querySelector('.git-deleted')"), 'Deleted file not listed')
    js(f"{files}.querySelector('.git-deleted').click()")
    wait_for(lambda: js(f"return !!{active()}.querySelector('.diff-remove')"), 'Deleted file diff missing')
    assert js(f"return {active()}.querySelector('[data-mode=file]').disabled"), 'Deleted file offered an invalid file view'
    js(f"[...{files}.querySelectorAll('.file')].find(row=>row.querySelector('.label').textContent==='hello world.txt').click()")
    assert js(f"return {active()}===window.docRoot && {active()}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Returning to file lost cached content'
    js(f"window.treeWidth={files}.querySelector('.file-browser').getBoundingClientRect().width;{files}.querySelector('.file-split').dispatchEvent(new KeyboardEvent('keydown',{{key:'ArrowLeft',bubbles:true}}))")
    assert js(f"return {files}.querySelector('.file-browser').getBoundingClientRect().width>window.treeWidth"), 'Internal split is not resizable'
    # A late read from an earlier click must not replace the current document.
    js(f"window.fileFetch=window.fetch;window.heldFile=null;window.fetch=async(input,...args)=>{{const u=new URL(input.url||input,location.href);if(u.pathname==='/workspace/file'&&u.searchParams.get('path').endsWith('delayed.txt'))return new Promise(resolve=>window.heldFile=()=>resolve(new Response(JSON.stringify({{kind:'text',text:'STALE FILE'}}),{{headers:{{'Content-Type':'application/json'}}}})));return fileFetch(input,...args)}};[...{files}.querySelectorAll('.file')].find(row=>row.querySelector('.label').textContent==='delayed.txt').click()")
    wait_for(lambda: js("return !!window.heldFile"), 'Deferred file read did not start')
    js(f"[...{files}.querySelectorAll('.file')].find(row=>row.querySelector('.label').textContent==='hello world.txt').click();window.heldFile()")
    wait_for(lambda: js(f"return {active()}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Late read replaced current file')
    js("window.fetch=window.fileFetch")
    js("window.tabCount=document.querySelectorAll('.workspace-tab').length;window.savedFetch=window.fetch;window.toggleFetches=0;window.fetch=(...args)=>{window.toggleFetches++;return window.savedFetch(...args)};for(let i=0;i<20;i++)document.querySelector('[data-workspace-toggle]').click();window.fetch=window.savedFetch")
    assert js(f"return window.toggleFetches===0 && {active()}===window.docRoot && document.querySelector('.session-workspace')===window.keptChat"), 'Hide/reveal caused requests or remounted content'
    js("document.querySelector('.workspace-tabbar [aria-label=\"Развернуть панель\"]').click()")
    assert js("return document.querySelector('.tab-workspace').classList.contains('expanded')"), 'Expand failed'
    js("document.querySelector('.workspace-tabbar [aria-label=\"Развернуть панель\"]').click()")
    # Closing the Files tab hides its workspace; reopening keeps tree and content.
    js("document.querySelector('.workspace-tab.active .workspace-tab-close').click()")
    choose('files')
    assert js(f"return {active()}===window.docRoot && {active()}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Closing Files discarded its browser state'
    Path('/tmp/proteus-inline-files.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    choose('model-quota')
    js("document.querySelector('.workspace-tab.active .workspace-tab-close').click()")
    choose('model-quota')
    assert js("return document.querySelector('[data-extension-id=model-quota]')===window.keptQuota"), 'Closing/reopening extension reset its runtime'
    Path('/tmp/proteus-tab-workspace.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    command('/window/rect', {'width': 760, 'height': 900})
    assert js("const r=document.querySelector('.tab-workspace').getBoundingClientRect();const chat=document.querySelector('.workspace-main').getBoundingClientRect(),side=document.querySelector('.sidebar').getBoundingClientRect();return r.left>=chat.right && chat.left>=side.right && r.top===chat.top && getComputedStyle(document.querySelector('.tab-workspace')).position==='relative' && getComputedStyle(document.querySelector('.app-layout')).flexDirection==='row'"), 'Workspace switched to mobile layout'
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    assert js("return !document.querySelector('.tab-workspace').hidden"), 'Narrow viewport still enables mobile Escape handling'
    command('/window/rect', {'width': 1440, 'height': 1000})
    js("for(const b of [...document.querySelectorAll('.workspace-tab-close')])b.click()")
    assert js("return !document.querySelector('.workspace-empty').hidden && document.querySelector('.workspace-empty [data-open-tab=usage]')"), 'Closing final tab lost the chooser'
    print('PASS: gray palette; header sidebar control; inline file/tree split; diff; cached switching and reopening; no refetch on toggle; desktop panels at narrow width', flush=True)
