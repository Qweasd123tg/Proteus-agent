"""Independent resizable columns, compact widgets and tabbed file preview."""
import json
import base64
from pathlib import Path


def run(command, js, wait_for):
    def card(id):
        return f"document.querySelector('[data-extension-id=\"{id}\"]')"

    def shadow(id):
        return f"{card(id)}?.querySelector('.extension-panel-content')?.shadowRoot"

    def column(id):
        return f"document.querySelector('aside.extension-column[data-column-id=\"{id}\"]')"

    command('/window/rect', {'width': 1920, 'height': 1000})
    files, preview = shadow('files'), shadow('files:preview')
    wait_for(lambda: js(f"return !!{files}?.querySelector('.file')"), 'File tree did not load')
    js(f"if({column('files')}.classList.contains('collapsed')){card('files')}.querySelector('.extension-compact').click()")
    js(f"[...{files}.querySelectorAll('.folder')].find(node=>node.querySelector('.label').textContent==='preview-fixture').click()")
    wait_for(lambda: js(f"return [...{files}.querySelectorAll('.file')].some(node=>node.querySelector('.label').textContent==='hello world.txt')"), 'Directory did not expand')
    js(f"[...{files}.querySelectorAll('.file')].find(node=>node.querySelector('.label').textContent==='hello world.txt').click()")
    wait_for(lambda: js(f"return !!{preview}?.querySelector('pre')?.textContent.includes('<b>Привет</b>')"), 'Separate file preview missing')
    assert js(f"return !{preview}.querySelector('pre b')"), 'File content interpreted as HTML'
    assert js(f"return !!{preview}.querySelector('.tab.transient')"), 'Single click did not open a transient preview tab'
    js(f"{files}.querySelector('.file.active').dispatchEvent(new MouseEvent('dblclick',{{bubbles:true}}))")
    assert js(f"return !{preview}.querySelector('.tab.transient')"), 'Double click did not pin file preview'
    wait_for(lambda: js(f"return !!{files}.querySelector('.file.active.git-modified .git-mark')"), 'Modified Git status did not reach file tree')
    assert js(f"return {files}.querySelector('.file.active .git-mark').textContent==='M'"), 'Modified file status marker missing'
    js(f"{preview}.querySelector('[data-mode=diff]').click()")
    wait_for(lambda: js(f"return [...{preview}.querySelectorAll('.diff-add')].some(line=>line.textContent.includes('<b>Привет</b>')) && [...{preview}.querySelectorAll('.diff-remove')].some(line=>line.textContent.includes('<b>Старое</b>'))"), 'Git preview did not render added and removed lines')
    assert js(f"return !{preview}.querySelector('pre b')"), 'Diff content interpreted as HTML'
    Path('/tmp/proteus-ui-columns.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js(f"{preview}.querySelector('[data-mode=file]').click()")
    assert js(f"return {preview}.querySelector('pre').textContent.includes('<b>Привет</b>') && !{preview}.querySelector('.diff-line')"), 'File view did not restore after diff'
    wait_for(lambda: js(f"return !!{files}.querySelector('.git-deleted[data-path=\"preview-fixture/deleted.txt\"]')"), 'Deleted file missing from tree changes')
    js("window.deletedFileReads=0;window.beforeDeletedFetch=window.fetch;window.fetch=(input,...args)=>{const url=new URL(typeof input==='string'?input:input.url,location.href);if(url.pathname.endsWith('/workspace/file')&&url.searchParams.get('path')==='preview-fixture/deleted.txt')window.deletedFileReads++;return window.beforeDeletedFetch(input,...args)}")
    js(f"{files}.querySelector('.git-deleted[data-path=\"preview-fixture/deleted.txt\"]').click()")
    wait_for(lambda: js(f"return [...{preview}.querySelectorAll('.diff-remove')].some(line=>line.textContent.includes('Удалённая строка'))"), 'Deleted file did not open its removal diff')
    assert js(f"return {preview}.querySelector('[data-mode=file]').disabled && window.deletedFileReads===0"), 'Deleted preview attempted to read a missing file'
    js("window.fetch=window.beforeDeletedFetch")
    js(f"[...{files}.querySelectorAll('.file')].find(row=>row.querySelector('.label').textContent==='hello world.txt').click()")
    assert js(f"return {preview}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Original file tab did not restore after deleted diff'
    assert js(f"return !{files}.querySelector('pre') && !!document.querySelector('[data-extension-columns=right] aside[data-column-id=\"files:preview\"]')"), 'Preview is not an independent right column'
    assert js("return getComputedStyle(document.querySelector('.session-workspace')).display!=='none' && getComputedStyle(document.querySelector('.sidebar')).display!=='none'"), 'File columns replaced chat or sidebar'
    js("window.keptChat=document.querySelector('.session-workspace');window.keptSidebar=document.querySelector('.sidebar')")
    assert js(f"return {column('files')}!=={column('files:preview')} && !document.querySelector('[data-extension-location] [data-extension-id=\"files\"]')"), 'Own panels share a widget dock'
    assert js(f"return [...{files}.querySelectorAll('.row')].every(row=>getComputedStyle(row).whiteSpace==='nowrap' && row.getBoundingClientRect().height<=28)"), 'File rows are not compact single lines'
    js(f"{files}.querySelector('.toolbar button').click()")
    wait_for(lambda: js(f"return !!{files}.querySelector('.file.active')"), 'Refresh lost expansion or selection')
    assert js(f"return {files}.querySelector('.file.active .label').textContent==='hello world.txt'"), 'Refresh changed selection'
    js(f"const row={files}.querySelector('.file.active');row.focus();row.dispatchEvent(new KeyboardEvent('keydown',{{key:'ArrowLeft',bubbles:true}}))")
    assert js(f"return {files}.activeElement.classList.contains('folder') && {files}.activeElement.querySelector('.label').textContent==='preview-fixture'"), 'Tree keyboard parent navigation failed'
    js(f"window.secondFile=[...{files}.querySelectorAll('.file:not(:disabled)')].find(node=>node.querySelector('.label').textContent!=='hello world.txt');if(window.secondFile)window.secondFile.click()")
    wait_for(lambda: js(f"return {preview}.querySelectorAll('[role=tab]').length===2"), 'Second preview tab missing')
    js(f"[...{preview}.querySelectorAll('[role=tab]')].find(tab=>tab.textContent==='hello world.txt').click()")
    assert js(f"return {preview}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Switching tabs lost loaded text'
    js(f"{preview}.querySelector('.tab:not(.active) .close').click()")
    assert js(f"return {preview}.querySelectorAll('[role=tab]').length===1 && {preview}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Closing inactive tab changed selection'
    js(f"window.keptFiles={card('files')};window.keptPreview={card('files:preview')};window.keptQuota={card('model-quota')};window.extensionMutations=0;window.extensionObserver=new MutationObserver(records=>window.extensionMutations+=records.filter(r=>[...r.removedNodes].includes(window.keptQuota)).length);window.extensionObserver.observe(document.querySelector('[data-extension-location=right]'),{{childList:true,subtree:true}})")

    def move(location):
        js(f"const select=window.keptFiles.querySelector('.extension-placement');select.value={json.dumps(location)};select.dispatchEvent(new Event('change'))")
        wait_for(lambda: js(f"return document.querySelector('[data-extension-columns={location}] aside[data-column-id=\"files\"] [data-extension-id=\"files\"]')===window.keptFiles"), 'Moving panel replaced the instance')
        assert js(f"return {card('files:preview')}===window.keptPreview && {preview}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Moving tree remounted its preview'

    assert js("return getComputedStyle(window.keptChat).display!=='none'"), 'File column replaced chat'
    js("window.keptFiles.querySelector('.extension-panel-title').click()")
    assert js("return document.getAnimations().some(a=>a.id==='extension-column')") != js("return matchMedia('(prefers-reduced-motion: reduce)').matches"), 'Column animation does not match motion preference'
    assert js(f"return {column('files')}.classList.contains('collapsed') && {column('files')}.getBoundingClientRect().width<=48"), 'Column did not collapse to a compact rail'
    assert js("return getComputedStyle(window.keptChat).display!=='none'"), 'Collapsing column hid chat'
    js("window.keptFiles.querySelector('.extension-compact').click()")
    assert js(f"return !{column('files')}.classList.contains('collapsed') && {column('files')}.getBoundingClientRect().width>=200"), 'Compact control did not restore column'
    move('right'); move('left')
    js(f"window.columnBefore={column('files')}.getBoundingClientRect().width;window.previewBefore={column('files:preview')}.getBoundingClientRect().width")
    point = js(f"const r={column('files')}.querySelector('.extension-column-resize').getBoundingClientRect();return {{x:Math.round(r.left+r.width/2),y:Math.round(r.top+r.height/2)}}")
    command('/actions', {'actions': [{'type': 'pointer', 'id': 'column-mouse', 'parameters': {'pointerType': 'mouse'}, 'actions': [
        {'type': 'pointerMove', 'duration': 0, 'x': point['x'], 'y': point['y']},
        {'type': 'pointerDown', 'button': 0},
        {'type': 'pointerMove', 'duration': 150, 'x': point['x'] + 80, 'y': point['y']},
        {'type': 'pointerUp', 'button': 0},
    ]}]})
    wait_for(lambda: js(f"return Math.abs({column('files')}.getBoundingClientRect().width-window.columnBefore)>20"), 'Dragging Files resize handle did not change width')
    assert js(f"return Math.abs({column('files:preview')}.getBoundingClientRect().width-window.previewBefore)<2 && document.querySelector('.session-workspace')===window.keptChat && document.querySelector('.sidebar')===window.keptSidebar"), 'Column resize changed preview width or remounted chat/sidebar'
    result = command('/execute/async', {'script': '''
      const done=arguments[arguments.length-1], card=window.keptQuota;
      const root=card.querySelector('.extension-panel-content'), title=card.querySelector('.extension-panel-title');
      let fetches=0;const original=window.fetch;window.fetch=(...args)=>{fetches++;return original(...args)};
      const start=performance.now();for(let i=0;i<20;i++)title.click();
      requestAnimationFrame(()=>{window.fetch=original;done({ms:performance.now()-start,fetches,same:root===card.querySelector('.extension-panel-content'),mutations:window.extensionMutations})});
    ''', 'args': []})
    print('EXTENSION_TOGGLE', json.dumps(result), flush=True)
    assert result['same'] and result['fetches'] == 0 and result['mutations'] == 0, 'Toggle remounted, fetched or detached another panel'
    animated = command('/execute/async', {'script': '''
      const done=arguments[arguments.length-1], title=window.keptQuota.querySelector('.extension-panel-title');
      const reveal=window.keptQuota.querySelector('.extension-panel-reveal');
      requestAnimationFrame(()=>{title.click();requestAnimationFrame(()=>{
        const running=reveal.getAnimations().length>0;
        title.click();done(running);
      })});
    ''', 'args': []})
    assert animated != js("return matchMedia('(prefers-reduced-motion: reduce)').matches"), 'Widget expansion has no local animation or ignores reduced motion'
    js('window.extensionObserver.disconnect()')
    assert js("return document.querySelector('[data-extension-id=model-quota] .extension-compact').getAttribute('aria-label').includes('37%')"), 'Weekly compact ring missing'
    js("if(document.querySelector('.info-panel.open'))[...document.querySelectorAll('[data-panel-toggle=info]')].find(b=>!b.closest('[inert]')).click()")
    assert js("return document.querySelector('[data-extension-id=model-quota] .extension-compact').getBoundingClientRect().width>0"), 'Compact extension hidden in rail'
    js("document.querySelector('[data-extension-id=model-quota] .extension-compact').click()")
    wait_for(lambda: js("return !!document.querySelector('.info-panel.open')"), 'Compact view did not open its dock')
    js(f"{preview}.querySelector('.close').click()")
    assert js(f"return {preview}.querySelectorAll('[role=tab]').length===0"), 'Last preview tab did not close'
    js(f"[...{files}.querySelectorAll('.file')].find(row=>row.querySelector('.label').textContent==='hello world.txt').click()")
    wait_for(lambda: js(f"return {preview}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'Closed preview did not reopen')
    assert js(f"return {card('files:preview')}===window.keptPreview"), 'Reopening preview remounted pane'
    js("document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=files] input')"), 'File extension settings missing')
    js("document.querySelector('[data-extension-choice=files] input').click()")
    wait_for(lambda: js("return !window.keptFiles.isConnected && !window.keptPreview.isConnected"), 'Disabling Files did not detach owner and preview')
    assert js(f"return !{card('files')} && !{card('files:preview')}"), 'Disabled file panes remained mounted'
    js("document.querySelector('[data-extension-choice=files] input').click()")
    wait_for(lambda: js(f"return !!{files}?.querySelector('.file') && {card('files')}!==window.keptFiles"), 'Re-enabling Files did not mount a fresh tree')
    assert js(f"return !{card('files:preview')}"), 'Preview eagerly remounted after enabling Files'
    js("document.querySelector('.topnav a[href=\"/\"]').click()")
    wait_for(lambda: js("return getComputedStyle(document.querySelector('.session-workspace')).display!=='none'"), 'Owner lifecycle check did not restore chat')
    js("document.querySelector('.sidebar-view-tabs button:first-child').click()")
    command('/window/rect', {'width': 1440, 'height': 1000})
