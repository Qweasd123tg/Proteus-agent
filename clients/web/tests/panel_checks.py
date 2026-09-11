"""Persistent docks, compact views and workspace preview through the real HTTP transport."""
import json


def run(command, js, wait_for):
    def shadow(id):
        return f"document.querySelector('[data-extension-id={id}] .extension-panel-content').shadowRoot"
    wait_for(lambda: js(f"return !!{shadow('files')}.querySelector('.file')"), 'File tree did not load')
    js("document.querySelector('.sidebar-view-tabs button:last-child').click()")
    js(f"[...{shadow('files')}.querySelectorAll('summary')].find(node=>node.textContent==='preview-fixture').click()")
    wait_for(lambda: js(f"return [...{shadow('files')}.querySelectorAll('.file')].some(node=>node.textContent==='hello world.txt')"), 'Directory did not expand')
    js(f"[...{shadow('files')}.querySelectorAll('.file')].find(node=>node.textContent==='hello world.txt').click()")
    wait_for(lambda: js(f"return {shadow('files')}.querySelector('pre').textContent.includes('<b>Привет</b>')"), 'File preview missing')
    assert js(f"return !{shadow('files')}.querySelector('pre b')"), 'File content interpreted as HTML'
    js("window.keptFiles=document.querySelector('[data-extension-id=files]');window.keptQuota=document.querySelector('[data-extension-id=model-quota]');window.extensionMutations=0;window.extensionObserver=new MutationObserver(records=>window.extensionMutations+=records.filter(r=>[...r.removedNodes].includes(window.keptQuota)).length);window.extensionObserver.observe(document.querySelector('[data-extension-location=right]'),{childList:true})")
    def move(location):
        js(f"const select=document.querySelector('[data-extension-id=files] .extension-placement');select.value={json.dumps(location)};select.dispatchEvent(new Event('change'))")
        wait_for(lambda: js(f"return document.querySelector('[data-extension-location={location}] [data-extension-id=files]')===window.keptFiles"), 'Moving panel replaced the instance')
    move('main')
    assert js(f"return {shadow('files')}.querySelector('pre').textContent.includes('<b>Привет</b>') && getComputedStyle(document.querySelector('.session-workspace')).display==='none'"), 'Main panel did not retain preview or hide chat'
    js("document.querySelector('[data-extension-id=files] .extension-panel-title').click()")
    assert js("return getComputedStyle(document.querySelector('.session-workspace')).display!=='none'"), 'Collapsed main panel did not return to chat'
    js("document.querySelector('[data-extension-id=files] .extension-compact').click()")
    js("document.querySelector('.topnav a').click()")
    assert js("return getComputedStyle(document.querySelector('.session-workspace')).display!=='none'"), 'Chat navigation did not dismiss main panel'
    js("document.querySelector('[data-extension-id=files] .extension-compact').click()")
    move('right'); move('left')
    result = command('/execute/async', {'script': '''
      const done=arguments[arguments.length-1], card=window.keptQuota;
      const root=card.querySelector('.extension-panel-content'), title=card.querySelector('.extension-panel-title');
      let fetches=0;const original=window.fetch;window.fetch=(...args)=>{fetches++;return original(...args)};
      const start=performance.now();for(let i=0;i<20;i++)title.click();
      requestAnimationFrame(()=>{window.fetch=original;done({ms:performance.now()-start,fetches,same:root===card.querySelector('.extension-panel-content'),mutations:window.extensionMutations})});
    ''', 'args': []})
    print('EXTENSION_TOGGLE', json.dumps(result), flush=True)
    assert result['same'] and result['fetches'] == 0 and result['mutations'] == 0, 'Toggle remounted, fetched or detached another panel'
    js('window.extensionObserver.disconnect()')
    assert js("return document.querySelector('[data-extension-id=model-quota] .extension-compact').getAttribute('aria-label').includes('37%')"), 'Weekly compact ring missing'
    js("if(document.querySelector('.info-panel.open'))[...document.querySelectorAll('[data-panel-toggle=info]')].find(b=>!b.closest('[inert]')).click()")
    assert js("return document.querySelector('[data-extension-id=model-quota] .extension-compact').getBoundingClientRect().width>0"), 'Compact extension hidden in rail'
    js("document.querySelector('[data-extension-id=model-quota] .extension-compact').click()")
    wait_for(lambda: js("return !!document.querySelector('.info-panel.open')"), 'Compact view did not open its dock')
    js("document.querySelector('.sidebar-view-tabs button:first-child').click()")
