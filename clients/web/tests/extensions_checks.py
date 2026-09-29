"""Browser regressions for settings/panel separation and extension lifetimes."""
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin, loaded):
    def settings():
        js("document.querySelector('.settings-link').click()")
        wait_for(lambda: js("return !!document.querySelector('[data-settings-section=chat]')"), 'Settings navigation missing')
        js("document.querySelector('[data-settings-section=chat]').click()")
        wait_for(lambda: js("return !!document.querySelector('[data-module-page=chat] input')"),'Chat settings missing')
        js("document.querySelector('[data-settings-section=extensions]').click()")
        wait_for(lambda: js("return !!document.querySelector('.extension-settings [data-extension-choice=notes]')"), 'Extension settings did not load')
        wait_for(lambda: js("return document.querySelector('.settings-link').classList.contains('active')"), 'Settings navigation highlight is stale')
    def chat():
        js("document.querySelector('.settings-back').click()")
        # Returning from Settings restores the selected tool; this helper needs chat.
        js("document.querySelector('.brand').click()")
        wait_for(loaded, 'Chat did not restore agent panel')
        wait_for(lambda: js("return document.querySelector('[data-tab-id=\"client:chat\"]').classList.contains('active')"), 'Chat navigation highlight is stale')
    def install(path):
        js("document.querySelector('[data-settings-section=extensions]').click()")
        wait_for(lambda: js("return !!document.querySelector('.extension-install')"),'Module manager missing')
        js("document.querySelector('.extension-source').open=true; document.querySelector('.extension-install input').value=location.origin+" + repr(path) + "; document.querySelector('.extension-install').requestSubmit()")
    def quota_loaded():
        return js("return document.querySelector('[data-extension-id=model-quota] .extension-panel-content')?.shadowRoot?.textContent.includes('73% осталось')")

    command('/url', {'url': web + '/standalone.html'})
    js("localStorage.setItem('proteus.toolCardsCollapsed','false');localStorage.setItem('proteus.ui.extensions', JSON.stringify({apiVersion:1,panels:[{id:'agent-info',url:location.origin+'/extensions/agent-info/extension.json',enabled:true,collapsed:false},{id:'notes',url:location.origin+'/extensions/notes/extension.json',enabled:false,collapsed:false}]}))")
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(loaded, 'Leptos transport did not deliver authenticated config')
    assert js("return document.querySelector('[data-client-view=settings]').hidden && !document.querySelector('[data-extension-id=model-quota]')")
    js("window.savedFetch=window.fetch; window.settingsWrites=0; window.fetch=(input,init)=>{const path=String(input.url||input).split('?')[0]; if(path.endsWith('/config/web'))window.settingsWrites++; return path.endsWith('/config')?Promise.resolve(new Response(JSON.stringify({error:'offline fixture'}),{status:502})):window.savedFetch(input,init)}")
    settings()
    assert js("return !document.querySelector('[data-module-page=chat] input').disabled"), 'Backend outage blocked client preferences'
    js("document.querySelector('[data-module-page=chat] input').click()")
    wait_for(lambda: js("return localStorage.getItem('proteus.toolCardsCollapsed') === 'true'"), 'Client preference was not persisted')
    assert js("return window.settingsWrites === 0"), 'Client setting called backend'
    js("window.savedSetItem=Storage.prototype.setItem; Storage.prototype.setItem=function(key,value){if(key==='proteus.toolCardsCollapsed')throw new Error('storage fixture');return window.savedSetItem.call(this,key,value)};document.querySelector('[data-module-page=chat] input').click()")
    wait_for(lambda: js("return document.querySelector('[data-module-page=chat] .settings-status').textContent.includes('Не сохранено') && document.querySelector('[data-module-page=chat] input').checked"), 'Storage failure was hidden or toggle did not roll back')
    js("Storage.prototype.setItem=window.savedSetItem;window.fetch=window.savedFetch")
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=chat]')"),'Settings navigation missing')
    js("document.querySelector('[data-settings-section=chat]').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=chat] input')"), 'Settings route did not restore')
    assert js("return document.querySelector('[data-module-page=chat] input').checked"), 'Client preference lost on reload'
    js("document.querySelector('[data-module-page=chat] input').click()")
    wait_for(lambda: js("return localStorage.getItem('proteus.toolCardsCollapsed') === 'false'"), 'Client preference did not reset')
    js("document.querySelector('[data-settings-section=extensions]').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-available=model-quota]')"), 'New bundled package unavailable in existing settings')
    js("document.querySelector('[data-extension-available=model-quota]').click(); document.querySelector('[data-extension-choice=notes] input').click(); document.querySelector('[data-reorder=notes]').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowUp',bubbles:true}))")
    # The live workspace mounts newly enabled extensions; opening Settings itself does not remount them.
    install('/fixture/extension.json')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=external-test]')"), 'External manifest not installed')
    wait_for(lambda: js('return window.externalMounted === 1'), 'Enabled extension did not mount')
    chat()
    wait_for(quota_loaded, 'Quota did not reach panel')
    wait_for(lambda: js('return window.externalMounted === 1'), 'External panel did not mount')
    assert js("const root=document.querySelector('[data-extension-id=model-quota] .extension-panel-content').shadowRoot; return root.textContent.includes('37% осталось') && root.textContent.includes('0% осталось') && root.textContent.includes('15 мин') && root.textContent.includes('Лимит исчерпан') && root.textContent.includes('ожидаем новые данные') && root.textContent.includes('12.50') && root.querySelectorAll('progress').length === 3")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')"), 'Notes absent')
    js("const area=document.querySelector('[data-extension-id=notes] .extension-panel-content').shadowRoot.querySelector('textarea');area.value='Моя заметка';area.dispatchEvent(new Event('input'))")
    assert js("return !!document.querySelector('[data-tab-id=notes]')"), 'Enabled tab missing'
    js("document.querySelector('[data-tab-id=external-test] .workspace-tab-close').click()")
    assert js('return !window.externalAborted && !window.externalDisposed')
    js("document.querySelector('.workspace-add').click();document.querySelector('.workspace-picker [data-open-tab=external-test]').click()")
    assert js('return window.externalMounted === 1'), 'Expand remounted a live panel'
    settings()
    assert js('return !window.externalAborted && !window.externalDisposed'), 'Navigation disposed a live dock'
    js("document.querySelector('[data-extension-choice=external-test] input').click()")
    chat()
    assert js('return window.externalMounted === 1 && window.externalAborted === 1 && window.externalDisposed === 1'), 'Disable did not stop exactly once'
    settings()
    js("document.querySelector('[data-extension-choice=external-test] input').click()")
    # Saved web setting must take effect on the next SPA chat mount.
    wait_for(lambda: js("return !document.querySelector('[data-module-page=chat] input').disabled"), 'Chat setting not ready')
    js("document.querySelector('[data-module-page=chat] input').click()")
    wait_for(lambda: js("return document.querySelector('[data-module-page=chat] .settings-status').textContent === 'Сохранено на этом устройстве'"), 'Chat setting save failed')
    assert js("return document.querySelector('[data-module-page=chat] input').checked")
    install('/fixture/slow/extension.json')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=slow-test]')"), 'Slow fixture not installed')
    chat()
    wait_for(lambda: js('return window.externalMounted === 2 && window.slowMounts === 1'), 'Return did not mount new panels')
    # Collapse while async mount is pending keeps that same instance.
    js("document.querySelector('[data-tab-id=slow-test] .workspace-tab-close').click();document.querySelector('.workspace-add').click();document.querySelector('.workspace-picker [data-open-tab=slow-test]').click()")
    assert js('return window.slowMounts === 1 && !window.slowDisposals'), 'Collapse restarted pending mount'
    js('window.finishSlowMount()')
    assert js('return !window.slowDisposals'), 'Live async instance was disposed'
    assert js("return document.querySelector('[data-extension-id=slow-test] .extension-panel-content').shadowRoot.textContent.includes('current panel')")
    settings()
    js("document.querySelector('[aria-label=\"Убрать: Медленная панель\"]').click(); document.querySelector('[data-extension-choice=external-test] input').click()")
    wait_for(lambda: js('return window.slowDisposals === 1'), 'Removal did not release async mount')
    chat()
    command('/refresh', {})
    wait_for(loaded, 'Reload failed')
    wait_for(lambda: js("return document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')?.value === 'Моя заметка'"), 'Notes lost on reload')
    assert js("return !!document.querySelector('[data-tab-id=notes]')"), 'Enabled tab missing'
    command('/url', {'url': web + '/standalone.html'})
    wait_for(lambda: js("return document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')?.value === 'Моя заметка'"), 'Independent host needs agent')
    wait_for(lambda: js("return document.querySelector('[data-extension-id=agent-info] .extension-error')?.textContent.includes('agent.config.read')"), 'Missing interface not isolated')
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(loaded, 'Final client load failed')
    wait_for(quota_loaded, 'Quota missing after reload')
    js("for(const id of ['agent-info','notes']){ document.querySelector(`[data-tab-id=${id}] .workspace-tab-close`)?.click() }")
    settings()
    for panel in ['plan','session-info','context','files','usage']:
        js(f"document.querySelector('[data-extension-available={panel}]')?.click()")
    chat()
    # Exercise the real composer / turn / tool card, after a setting changed via SPA.
    js("const area=document.querySelector('.composer textarea');area.value='Проверь интерфейс';area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Send action did not enable after input')
    js("document.querySelector('.composer-submit').click()")
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Проверка интерфейса завершена.')"), 'Composer / tool turn did not complete')
    assert js("return !!document.querySelector('.tool-card') && !document.querySelector('.tool-card.expanded')"), 'Saved compact-card setting did not apply to SPA chat'
    assert js("return !!document.querySelector('.code-block') && document.querySelector('[data-extension-id=plan] .extension-panel-content')?.shadowRoot?.textContent.includes('Проверить панели')"), 'Code block or plan panel did not render'
    assert js("const b=document.querySelector('[data-widget-id=plan]');return b?.dataset.uiTooltipDetails.includes('Проверить панели') && b.dataset.uiTooltipDetails.includes('Проверить настройки')"), 'Plan did not publish full hover steps'
    assert js("const b=document.querySelector('[data-widget-id=session-info]');return !!b?.dataset.uiTooltipDetails && !document.querySelector('[data-tab-id=session-info]') && !document.querySelector('[data-extension-id=session-info]')"), 'Session info did not remain compact-only'
