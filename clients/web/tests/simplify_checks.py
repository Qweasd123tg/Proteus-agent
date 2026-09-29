"""Navigation, stable footer and cached Inspector views against the real local API."""
import base64
from pathlib import Path
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin):
    # The transport goes through Connecting again without changing footer geometry.
    js("window.footerBefore=[...document.querySelectorAll('.sidebar-footer > a')].map(x=>x.getBoundingClientRect().y);window.footerMoves=[];window.footerObserver=new ResizeObserver(()=>footerMoves.push(document.querySelector('.sidebar-footer').getBoundingClientRect().height));footerObserver.observe(document.querySelector('.sidebar-footer'));document.querySelector('.connection-badge').click()")
    wait_for(lambda: js("return document.querySelector('.connection-badge').classList.contains('completed')"), 'Reconnect did not finish')
    assert js("return JSON.stringify(footerBefore)===JSON.stringify([...document.querySelectorAll('.sidebar-footer > a')].map(x=>x.getBoundingClientRect().y)) && new Set(footerMoves).size<=1"), 'Connecting moved navigation'
    js("footerObserver.disconnect();window.workspaceBefore=document.querySelector('.tab-workspace');window.sessionBefore=new URL(location.href).searchParams.get('session_dir');const area=document.querySelector('.composer textarea');area.value='Сохранённый черновик';area.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('.settings-back')"), 'Settings missing')
    assert js("return !document.querySelector('.sidebar-footer a[href=\"/context\"]') && document.querySelector('.sidebar').getBoundingClientRect().width>0"), 'Diagnostics still clutter chat navigation'
    js("document.querySelector('[data-settings-section=extensions]').click()")
    wait_for(lambda: js("return document.querySelector('[data-settings-section=extensions]').getAttribute('aria-pressed')==='true'"), 'Settings selection is stale')
    assert js("return !document.querySelector('[data-module-page=extensions]').hidden && document.querySelector('.settings-section:has(.appearance-preview)').hidden"), 'Settings sections overlap'
    Path('/tmp/proteus-settings-simplified.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    for width in [900,620,390]:
        command('/window/rect', {'width':width,'height':1000})
        assert js("return document.documentElement.scrollWidth<=Math.max(innerWidth,860)"), 'Settings horizontal overflow'
    command('/window/rect', {'width':1440,'height':1000})
    js("document.querySelector('[data-settings-section=diagnostic-usage]').click()")
    assert js("return !document.querySelector('a[href=\"/context\"], a[href=\"/resume\"]') && document.querySelectorAll('[data-settings-section^=diagnostic-]').length===4 && document.querySelectorAll('.settings-nav button svg').length===8"), 'Diagnostics, history or settings icons are wrong'
    js("document.querySelector('.settings-back').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"), 'Return to chat failed')
    assert js("return document.querySelector('.composer textarea').value==='Сохранённый черновик' && new URL(location.href).searchParams.get('session_dir')===sessionBefore && document.querySelector('.tab-workspace')===workspaceBefore"), 'Settings lost chat, draft or tool tabs'
    js("const area=document.querySelector('.composer textarea');area.value='';area.dispatchEvent(new Event('input',{bubbles:true}))")
    print('PASS: stable connection footer; settings sections; diagnostics placement; draft/session/tool tabs retained; responsive settings',flush=True)

    js("const area=document.querySelector('.composer textarea');area.value='Проверь интерфейс';area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Send disabled')
    js("document.querySelector('.composer-submit').click()")
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Проверка интерфейса завершена.')"), 'Fixture turn did not complete')
    js("window.savedSession=new URL(location.href).searchParams.get('session_dir');document.querySelector('[aria-label=\"Новая сессия\"]').click()")
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')!==savedSession && document.querySelector('.connection-badge').classList.contains('completed')"), 'New session did not connect')
    wait_for(lambda: js("return [...document.querySelectorAll('.session-history-item')].some(x=>x.textContent.includes('Проверь интерфейс'))"), 'Saved session missing from sidebar')
    js("[...document.querySelectorAll('.session-history-item')].find(x=>x.textContent.includes('Проверь интерфейс')).click()")
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')===savedSession && document.querySelector('.connection-badge').classList.contains('completed')"), 'Saved session did not reopen')
    assert js("return JSON.stringify(footerBefore)===JSON.stringify([...document.querySelectorAll('.sidebar-footer > a')].map(x=>x.getBoundingClientRect().y))"), 'Session switch moved footer'

    command('/url', {'url':web+'/architecture?'+urlencode({'server':origin,'token':'extension-smoke'})})
    wait_for(lambda: js("return !!document.querySelector('[data-node-id=\"slot:workflow\"]')"), 'Inspector architecture missing')
    js("window.keptGraph=document.querySelector('.graph-viewport');window.inspectorReads=[];window.inspectorFetch=window.fetch;window.fetch=(input,...args)=>{inspectorReads.push(new URL(input.url||input,location.href).pathname);return inspectorFetch(input,...args)};document.querySelector('.inspector-nav-item[href*=\"view=configs\"]').click()")
    wait_for(lambda: js("return !!document.querySelector('.cfg-tabs')"), 'Builder did not open')
    js("window.keptBuilder=document.querySelector('.cfg-tabs');document.querySelector('.inspector-nav-item[href*=\"view=analysis\"]').click()")
    wait_for(lambda: js("return !!document.querySelector('.turn-analysis')"), 'Turn analysis did not open')
    wait_for(lambda: js("return !!document.querySelector('.analysis-turn-summary')"), 'Real journal did not load')
    for _ in range(3):
        js("document.querySelector('.inspector-nav-item[href*=\"view=architecture\"]').click();document.querySelector('.inspector-nav-item[href*=\"view=configs\"]').click()")
    assert js("return document.querySelector('.cfg-tabs')===keptBuilder && document.querySelector('.graph-viewport')===keptGraph && !inspectorReads.includes('/resume') && !inspectorReads.includes('/new-session')"), 'Inspector reloaded views or resumed session on navigation'
    js("history.back()")
    wait_for(lambda: js("return document.querySelector('.inspector-nav-item.active').textContent.includes('Архитектура')"), 'Inspector Back failed')
    assert js("return !document.querySelector('.graph-viewport').closest('.inspector-view').hidden && document.querySelectorAll('.inspector-view:not([hidden])').length===1"), 'Hidden Inspector views affect layout'
    Path('/tmp/proteus-inspector-simplified.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('[data-node-id=\"slot:workflow\"]')"), 'Inspector route not restored on reload')
    print('PASS: Inspector in-app navigation; cached graph/builder; no repeated session initialization; Back and reload',flush=True)
