"""Live demo of a disabled package: explicit start, demo data, no writes, teardown."""


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
       "for(const id of ['plan','notes'])if(demoEnabled[id])document.querySelector('[data-extension-choice='+id+'] input').click()")
    wait_for(lambda: js("return !document.querySelector('[data-settings-section=plan]') && !document.querySelector('[data-widget-id=plan]')"), 'Plan did not turn off')
    js("window.demoStorage=()=>JSON.stringify(Object.entries(localStorage).filter(([k])=>k.startsWith('proteus.ui.extension.')).sort());window.savedDemoStorage=demoStorage()")

    page('plan')
    wait_for(lambda: js("return !!document.querySelector('[data-preview-try=plan]')"), 'Disabled package has no try action')
    assert js("return !document.querySelector('[data-extension-demo]')"), 'Opening the page ran the package'
    click('[data-preview-try=plan]')
    wait_for(lambda: js(f"return !!{shadow('plan')}?.textContent.includes('Изучить структуру проекта')"), 'Demo did not render the plan')
    wait_for(lambda: js("return /\\d\\/5/.test(document.querySelector('[data-extension-demo=plan] .extension-widget span')?.shadowRoot?.textContent||'')"), 'Demo widget did not count the plan')
    assert js("return !document.querySelector('[data-extension-toggle=plan]').checked && !document.querySelector('[data-widget-id=plan]') && !document.querySelector('[data-tab-id=plan]')"), 'Demo enabled the package or reached the workspace'
    click('[data-extension-demo=plan] .extension-demo-close')
    assert js("return !document.querySelector('[data-extension-demo]') && document.activeElement?.dataset.previewTry==='plan'"), 'Close kept the demo or lost focus'

    # Text typed into the demo stays in its memory; enabling replaces the demo.
    page('notes')
    click('[data-preview-try=notes]')
    wait_for(lambda: js(f"return !!{shadow('notes')}?.querySelector('textarea')"), 'Notes demo missing')
    js(f"const t={shadow('notes')}.querySelector('textarea');t.value='демо-заметка';t.dispatchEvent(new Event('input'))")
    assert js(f"return {shadow('notes')}.textContent.includes('Сохранено') && demoStorage()===savedDemoStorage"), 'Demo wrote extension storage'
    click('[data-extension-toggle=notes]')
    wait_for(lambda: js("return !document.querySelector('[data-extension-demo]') && !!document.querySelector('[data-widget-id=notes]')"), 'Enabling kept the demo')
    assert js("return !localStorage.getItem('proteus.ui.extension.notes:text')?.includes('демо')"), 'Demo text reached the real extension'

    click('[data-settings-section=extensions]')
    js("for(const id of ['plan','notes']){const i=document.querySelector('[data-extension-choice='+id+'] input');if(i.checked!==demoEnabled[id])i.click()}")
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=plan]')===demoEnabled.plan && !!document.querySelector('[data-settings-section=notes]')===demoEnabled.notes"), 'Extension states not restored')
    click('.settings-back')
    print('PASS: disabled package demo runs only on request; demo data and widget; nothing saved or enabled; close/enable teardown', flush=True)
