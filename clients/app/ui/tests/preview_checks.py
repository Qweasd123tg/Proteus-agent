"""Automatic package-owned previews: enabled/disabled, no writes or autofocus, teardown."""


def run(command, js, wait_for):
    def click(selector):
        element = command('/element', {'using': 'css selector', 'value': selector})
        command('/element/' + element['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    def page(id):
        click('[data-settings-section=extensions]')
        wait_for(lambda: js(f"return !!document.querySelector('[data-settings-id={id}]')"), f'{id} missing from management')
        click(f'[data-settings-id={id}]')

    def shadow(id):
        return f"document.querySelector('[data-extension-demo={id}] .extension-panel-content')?.shadowRoot"

    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"), 'Settings missing')
    click('[data-settings-section=extensions]')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=notes] input')"), 'Management list missing')
    js("window.demoEnabled=Object.fromEntries(['plan','notes'].map(id=>[id,document.querySelector('[data-extension-choice='+id+'] input').checked]));"
       "if(!demoEnabled.plan)document.querySelector('[data-extension-choice=plan] input').click();"
       "if(demoEnabled.notes)document.querySelector('[data-extension-choice=notes] input').click()")
    js("window.demoStorage=()=>JSON.stringify(Object.entries(localStorage).filter(([k])=>k.startsWith('proteus.ui.extension.')).sort());window.savedDemoStorage=demoStorage()")

    page('plan')
    wait_for(lambda: js(f"return !!{shadow('plan')}?.textContent.includes('Изучить структуру проекта')"), 'Enabled package did not automatically preview its plan')
    wait_for(lambda: js("return /\\d\\/5/.test(document.querySelector('[data-extension-demo=plan] .extension-widget span')?.shadowRoot?.textContent||'')"), 'Demo widget did not count the plan')
    assert js("return document.querySelector('[data-extension-toggle=plan]').checked && !document.querySelector('.extension-preview img') && !document.activeElement?.closest('[data-extension-demo]')"), 'Preview changed enabled state, retained a placeholder picture or stole focus'
    js("window.keptPlanDemo=document.querySelector('[data-extension-demo=plan]')")
    click('[data-extension-toggle=plan]')
    wait_for(lambda: js("return !document.querySelector('[data-widget-id=plan]')"), 'Plan did not turn off')
    assert js("return keptPlanDemo===document.querySelector('[data-extension-demo=plan]') && keptPlanDemo.isConnected"), 'Disabling restarted or hid the preview'
    assert js("return !document.querySelector('[data-extension-toggle=plan]').checked && !document.querySelector('[data-widget-id=plan]') && !document.querySelector('[data-tab-id=plan]')"), 'Demo enabled the package or reached the workspace'

    page('notes')
    wait_for(lambda: js(f"return !!{shadow('notes')}?.querySelector('textarea')"), 'Notes demo missing')
    assert js("return !keptPlanDemo.isConnected"), 'Hidden page retained a running preview'
    js(f"const t={shadow('notes')}.querySelector('textarea');t.value='демо-заметка';t.dispatchEvent(new Event('input'))")
    assert js(f"return {shadow('notes')}.textContent.includes('Сохранено') && demoStorage()===savedDemoStorage"), 'Demo wrote extension storage'
    js("window.keptNotesDemo=document.querySelector('[data-extension-demo=notes]')")
    click('[data-extension-toggle=notes]')
    wait_for(lambda: js("return !!document.querySelector('[data-widget-id=notes]')"), 'Enabling did not mount the real widget')
    assert js(f"return keptNotesDemo===document.querySelector('[data-extension-demo=notes]') && {shadow('notes')}.querySelector('textarea').value==='демо-заметка'"), 'Enabling restarted or hid the preview'
    assert js("return !localStorage.getItem('proteus.ui.extension.notes:text')?.includes('демо')"), 'Demo text reached the real extension'

    click('[data-settings-section=extensions]')
    wait_for(lambda: js("return !document.querySelector('[data-extension-demo]')"), 'Leaving the page kept preview runtimes')
    page('plan')
    wait_for(lambda: js(f"return !!{shadow('plan')}?.textContent.includes('Изучить структуру проекта')"), 'Returning did not automatically restart the preview')
    assert js("return document.querySelector('[data-extension-demo=plan]')!==keptPlanDemo"), 'Returning reused a stopped demo'
    click('.settings-back')
    wait_for(lambda: js("return !document.querySelector('[data-extension-demo]')"), 'Leaving settings kept preview runtimes')
    click('.settings-link')
    wait_for(lambda: js(f"return !!{shadow('plan')}?.textContent.includes('Изучить структуру проекта')"), 'Reopening settings did not restart the preview')
    click('[data-settings-section=extensions]')
    js("for(const id of ['plan','notes']){const i=document.querySelector('[data-extension-choice='+id+'] input');if(i.checked!==demoEnabled[id])i.click()}")
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=plan]')===demoEnabled.plan && !!document.querySelector('[data-settings-section=notes]')===demoEnabled.notes"), 'Extension states not restored')
    click('.settings-back')
    print('PASS: automatic package-owned enabled/disabled previews; no autofocus; data/widget; in-memory edits; toggles preserve preview; hidden pages/settings release and restart runtimes', flush=True)
