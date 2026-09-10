"""Browser regressions for settings/panel separation and extension lifetimes."""
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin, loaded):
    def settings():
        js("document.querySelector('.settings-link').click()")
        wait_for(lambda: js("return !!document.querySelector('.extension-settings [data-extension-choice=notes]')"), 'Extension settings did not load')
        wait_for(lambda: js("return document.querySelector('.settings-link').classList.contains('active')"), 'Settings navigation highlight is stale')
    def chat():
        js("document.querySelector('.topnav a[href=\"/\"]').click()")
        wait_for(loaded, 'Chat did not restore agent panel')
        wait_for(lambda: js("return document.querySelector('.topnav a[href=\"/\"]').classList.contains('active')"), 'Chat navigation highlight is stale')
    def install(path):
        js("document.querySelector('.extension-source').open=true; document.querySelector('.extension-install input').value=location.origin+" + repr(path) + "; document.querySelector('.extension-install').requestSubmit()")
    def quota_loaded():
        return js("return document.querySelector('[data-extension-id=model-quota] .extension-panel-content')?.shadowRoot?.textContent.includes('73% осталось')")

    command('/url', {'url': web + '/standalone.html'})
    js("localStorage.setItem('proteus.ui.extensions', JSON.stringify({apiVersion:1,panels:[{id:'agent-info',url:location.origin+'/extensions/agent-info/extension.json',enabled:true,collapsed:false},{id:'notes',url:location.origin+'/extensions/notes/extension.json',enabled:false,collapsed:false}]}))")
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(loaded, 'Leptos transport did not deliver authenticated config')
    assert js("return !document.querySelector('.extension-manager') && !document.querySelector('.extension-install') && !document.querySelector('[data-extension-id=model-quota]')")
    js("window.savedFetch=window.fetch;window.fetch=(input,init)=>String(input.url||input).endsWith('/config')?Promise.resolve(new Response(JSON.stringify({error:'offline fixture'}),{status:502})):window.savedFetch(input,init)")
    settings()
    wait_for(lambda: js("return document.querySelector('.settings-status').textContent.includes('Не удалось загрузить')"), 'Initial settings failure was hidden')
    assert js("return document.querySelector('#general input').disabled && !document.querySelector('[data-extension-choice=notes] input').disabled"), 'Agent outage blocked local extension settings'
    js('window.fetch=window.savedFetch;document.querySelector(".settings-retry").click()')
    wait_for(lambda: js("return !document.querySelector('#general input').disabled"), 'Settings retry did not recover')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-available=model-quota]')"), 'New bundled package unavailable in existing settings')
    js("document.querySelector('[data-extension-available=model-quota]').click(); document.querySelector('[data-extension-choice=notes] input').click(); document.querySelector('[aria-label=\"Выше: Заметки\"]').click()")
    # Settings loads manifests, not executable panel code.
    install('/fixture/extension.json')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=external-test]')"), 'External manifest not installed')
    assert js('return !window.externalMounted'), 'Settings executed extension entry point'
    chat()
    wait_for(quota_loaded, 'Quota did not reach panel')
    wait_for(lambda: js('return window.externalMounted === 1'), 'External panel did not mount')
    js("if (!document.querySelector('.info-panel.open')) document.querySelector('.info-panel-header button').click()")
    assert js("const root=document.querySelector('[data-extension-id=model-quota] .extension-panel-content').shadowRoot; return root.textContent.includes('37% осталось') && root.textContent.includes('0% осталось') && root.textContent.includes('15 мин') && root.textContent.includes('Лимит исчерпан') && root.textContent.includes('ожидаем новые данные') && root.textContent.includes('12.50') && root.querySelectorAll('progress').length === 3")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')"), 'Notes absent')
    js("const area=document.querySelector('[data-extension-id=notes] .extension-panel-content').shadowRoot.querySelector('textarea');area.value='Моя заметка';area.dispatchEvent(new Event('input'))")
    assert js("return document.querySelector('.extension-panels').firstElementChild.dataset.extensionId") == 'notes'
    js("document.querySelector('[data-extension-id=external-test] .extension-panel-title').click()")
    assert js('return window.externalAborted === 1 && window.externalDisposed === 1')
    js("document.querySelector('[data-extension-id=external-test] .extension-panel-title').click()")
    wait_for(lambda: js('return window.externalMounted === 2'), 'Expand did not remount')
    settings()
    assert js('return window.externalAborted === 2 && window.externalDisposed === 2')
    js("document.querySelector('[data-extension-choice=external-test] input').click()")
    chat()
    assert js('return window.externalMounted === 2'), 'Disabled panel remounted'
    settings()
    js("document.querySelector('[data-extension-choice=external-test] input').click()")
    # Saved web setting must take effect on the next SPA chat mount.
    wait_for(lambda: js("return !document.querySelector('#general input').disabled"), 'Chat setting not ready')
    js("document.querySelector('#general input').click()")
    wait_for(lambda: js("return document.querySelector('.settings-status').textContent === 'Сохранено'"), 'Chat setting save failed')
    assert js("return document.querySelector('#general input').checked")
    # A failed save rolls the visual state back and leaves server setting intact.
    js("window.realFetch=window.fetch;window.fetch=(input,init)=>String(input.url||input).endsWith('/config/web')?Promise.resolve(new Response(JSON.stringify({error:'fixture failure'}),{status:502})):window.realFetch(input,init);document.querySelector('#general input').click()")
    wait_for(lambda: js("return document.querySelector('.settings-status').textContent.includes('Не сохранено')"), 'Failure did not reach settings')
    assert js("return document.querySelector('#general input').checked && !document.querySelector('#general input').disabled")
    js('window.fetch=window.realFetch')
    install('/fixture/slow/extension.json')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=slow-test]')"), 'Slow fixture not installed')
    chat()
    wait_for(lambda: js('return window.externalMounted === 3 && window.slowMounts === 1'), 'Return did not mount new panels')
    # Collapse while async mount is pending, then expand before its disposer returns.
    js("document.querySelector('[data-extension-id=slow-test] .extension-panel-title').click();document.querySelector('[data-extension-id=slow-test] .extension-panel-title').click()")
    wait_for(lambda: js('return window.slowMounts === 2'), 'Second async instance absent')
    js('window.finishSlowMount()')
    wait_for(lambda: js('return window.slowDisposals === 1'), 'Late disposer lost')
    assert js("return document.querySelector('[data-extension-id=slow-test] .extension-panel-content').shadowRoot.textContent.includes('current panel')")
    settings()
    js("document.querySelector('[aria-label=\"Убрать: Медленная панель\"]').click(); document.querySelector('[data-extension-choice=external-test] input').click()")
    chat()
    command('/refresh', {})
    wait_for(loaded, 'Reload failed')
    wait_for(lambda: js("return document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')?.value === 'Моя заметка'"), 'Notes lost on reload')
    assert js("return document.querySelector('.extension-panels').firstElementChild.dataset.extensionId") == 'notes'
    command('/url', {'url': web + '/standalone.html'})
    wait_for(lambda: js("return document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')?.value === 'Моя заметка'"), 'Independent host needs agent')
    wait_for(lambda: js("return document.querySelector('[data-extension-id=agent-info] .extension-error')?.textContent.includes('agent.config.read')"), 'Missing interface not isolated')
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(loaded, 'Final client load failed')
    wait_for(quota_loaded, 'Quota missing after reload')
    js("for(const id of ['agent-info','notes']){ const title=document.querySelector(`[data-extension-id=${id}] .extension-panel-title`); if(title?.getAttribute('aria-expanded')==='true')title.click() }")
    # Exercise the real composer / turn / tool card, after a setting changed via SPA.
    js("const area=document.querySelector('.composer textarea');area.value='Проверь интерфейс';area.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.composer').requestSubmit()")
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Проверка интерфейса завершена.')"), 'Composer / tool turn did not complete')
    assert js("return !!document.querySelector('.tool-card') && !document.querySelector('.tool-card.expanded')"), 'Saved compact-card setting did not apply to SPA chat'
    assert js("return !!document.querySelector('.code-block') && document.querySelector('.info-panel-body').textContent.includes('Проверить панели')"), 'Code block or plan panel did not render'
