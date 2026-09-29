"""Actual registry installation, service wiring, teardown and embedded diagnostics."""
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin, loaded):
    def click(selector):
        js('document.querySelector('+repr(selector)+').click()')
    def page(id):
        click('[data-settings-section='+id+']')
        wait_for(lambda: js('return !!document.querySelector("[data-module-page='+id+'] .client-module-content")?.children.length'), 'Module missing: '+id)
    command('/url', {'url':web+'/?'+urlencode({'server':origin,'token':'extension-smoke','inspector':web})})
    wait_for(loaded,'Client not connected')
    wait_for(lambda: js("return !!document.querySelector('.composer-model-menu')"),'Built-in selector not mounted')
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"),'Settings navigation missing')
    page('extensions')
    assert js("return document.querySelector('[data-builtin-module=extensions] input').disabled"),'Management can disable itself'
    click('[data-builtin-module=model-selector] input')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"),'Chat missing')
    assert js("return !document.querySelector('.composer-model-menu') && !!document.querySelector('.composer-access-menu')"),'Disabled module still mounted'
    click('.settings-link');page('extensions')
    click('[data-builtin-module=model-selector] input')
    js("document.querySelector('.extension-install input').value=location.origin+'/fixture/client/extension.json';document.querySelector('.extension-install').requestSubmit()")
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=client-test]')"),'Installed diagnostic absent from navigation')
    assert js("return !document.querySelector('[data-tab-id=client-test]') && !window.clientMounts"),'Settings-only module mounted in workspace'
    page('client-test')
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=client-test] [data-config-read]')"),'Declared agent service unavailable')
    js("document.querySelector('[data-custom-module=settings]').value='draft'")
    page('appearance');page('client-test')
    assert js("return clientMounts===1 && document.querySelector('[data-custom-module=settings]').value==='draft'"),'Navigation discarded custom page'
    page('extensions')
    click('[data-extension-choice=client-test] input')
    wait_for(lambda: js("return clientAborts===1 && clientDisposals===1 && !document.querySelector('[data-settings-section=client-test]')"),'Disable did not clean custom page')
    click('[data-extension-choice=client-test] input')
    click('[data-select-slot=composer-model][data-module-id=client-test]')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('[data-custom-module=composer-model]')"),'Alternative selector not mounted')
    assert js("return !document.querySelector('.composer-model-menu')"),'Two implementations selected at once'
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('[data-custom-module=composer-model]')"),'Selection not restored')
    click('.settings-link');page('extensions')
    click('[data-select-slot=composer-model][data-module-id=model-selector]')
    page('diagnostic-usage')
    wait_for(lambda: js("return !!document.querySelector('.diagnostic-frame')?.contentDocument?.querySelector('.inspector-shell')"),'Embedded Inspector not loaded')
    assert js("const f=document.querySelector('.diagnostic-frame'),d=f.contentDocument;return getComputedStyle(d.querySelector('.inspector-sidebar')).display==='none' && f.getBoundingClientRect().height>250 && !d.body.textContent.includes('Session token storage failed')"),'Embedded diagnostics chrome/connection failed'
    wait_for(lambda: js("return document.querySelector('.diagnostic-frame').contentDocument.querySelector('#analysis-session')?.options.length>0"),'Embedded report did not authenticate/load sessions')
    js("window.keptDiagnostic=document.querySelector('.diagnostic-frame')")
    page('appearance');page('diagnostic-usage')
    assert js("return keptDiagnostic===document.querySelector('.diagnostic-frame')"),'Diagnostic iframe remounted on switch'
    for id in ['diagnostic-analysis','diagnostic-configs','diagnostic-architecture']:
        page(id)
        wait_for(lambda: js('return !!document.querySelector("[data-module-page='+id+'] iframe")?.contentDocument?.querySelector(".inspector-shell")'), 'Diagnostic not loaded: '+id)
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=diagnostic-architecture] iframe').contentDocument.querySelector('[data-node-id=\"slot:workflow\"]')"),'Embedded architecture did not read real topology')
    page('extensions');click('[data-builtin-module=diagnostic-usage] input')
    assert js("return !keptDiagnostic.isConnected && !document.querySelector('[data-settings-section=diagnostic-usage]')"),'Disabled diagnostic retained iframe'
    click('[data-builtin-module=diagnostic-usage] input')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('.composer-model-menu')"),'Built-in selector not restored')
    click('.settings-link');page('diagnostic-usage')
    wait_for(lambda: js("return !!document.querySelector('.diagnostic-frame')?.contentDocument?.querySelector('.analysis-open-chat')"),'Return-to-chat action missing')
    js("document.querySelector('.diagnostic-frame').contentDocument.querySelector('.analysis-open-chat').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea') && !document.querySelector('.settings-page')"),'Embedded diagnostic did not return to parent chat')
    print('PASS: installed diagnostic and selector use declared services; lazy mounting; drafts; disable/dispose; required management; persistent replacement; all four embedded Inspector pages',flush=True)
