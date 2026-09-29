"""Actual registry installation, service wiring, teardown and embedded diagnostics."""
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin, loaded):
    def click(selector):
        # Built-in controls are intentionally under a closed disclosure.
        js('const target=document.querySelector('+repr(selector)+');const system=target.closest(".builtin-module-settings");if(system&&!system.open)system.querySelector("summary").click();target.click()')
    def page(id):
        click('[data-settings-section='+id+']')
        wait_for(lambda: js('return !!document.querySelector("[data-module-page='+id+'] .client-module-content")?.children.length'), 'Module missing: '+id)
    command('/url', {'url':web+'/?'+urlencode({'server':origin,'token':'extension-smoke','inspector':web})})
    wait_for(loaded,'Client not connected')
    wait_for(lambda: js("return !!document.querySelector('.composer-model-menu')"),'Built-in selector not mounted')
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"),'Settings navigation missing')
    page('extensions')
    assert js("const system=document.querySelector('details.builtin-module-settings');return system && !system.open && system.querySelector('summary').textContent==='Системные модули' && !!system.querySelector('[data-builtin-module]') && [...document.querySelectorAll('[data-extension-choice]')].every(row=>!system.contains(row))"),'System modules are not separate or are expanded by default'
    assert js("return document.querySelector('[data-builtin-module=extensions] input').disabled"),'Management can disable itself'
    click('[data-builtin-module=model-selector] input')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"),'Chat missing')
    assert js("return !document.querySelector('.composer-model-menu') && !!document.querySelector('.composer-access-menu')"),'Disabled module still mounted'
    click('.settings-link');page('extensions')
    assert js("return !document.querySelector('.builtin-module-settings').open"),'Leaving settings retained an open system disclosure'
    click('[data-builtin-module=model-selector] input')
    click('.extension-source > summary')
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
    wait_for(lambda: js("return !!document.querySelector('[data-client-slot=composer-model] [data-config-read] [data-custom-module=composer-model]')"),'Selection or declared service not restored')
    click('.settings-link');page('extensions')
    click('[data-select-slot=composer-model][data-module-id=model-selector]')
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'I',code:'KeyI',ctrlKey:true,shiftKey:true,bubbles:true,cancelable:true}))")
    wait_for(lambda: js("return document.querySelector('[data-settings-section=diagnostic-usage]')?.classList.contains('active') && !document.querySelector('[data-client-view=settings]').hidden"), 'Diagnostic shortcut did not select embedded settings')
    wait_for(lambda: js("return !!document.querySelector('.diagnostic-frame')?.contentDocument?.querySelector('.inspector-shell')"),'Embedded Inspector not loaded')
    assert js("const f=document.querySelector('.diagnostic-frame'),d=f.contentDocument;return getComputedStyle(d.querySelector('.inspector-sidebar')).display==='none' && f.getBoundingClientRect().height>250 && !d.body.textContent.includes('Session token storage failed')"),'Embedded diagnostics chrome/connection failed'
    wait_for(lambda: js("return document.querySelector('.diagnostic-frame').contentDocument.querySelector('#analysis-session')?.options.length>0"),'Embedded report did not authenticate/load sessions')
    js("window.keptDiagnostic=document.querySelector('.diagnostic-frame')")
    page('appearance');page('diagnostic-usage')
    assert js("return keptDiagnostic===document.querySelector('.diagnostic-frame')"),'Diagnostic iframe remounted on switch'
    js("window.keptDiagnosticDocument=keptDiagnostic.contentDocument;window.diagnosticLoads=0;keptDiagnostic.addEventListener('load',()=>diagnosticLoads++)")
    page('appearance');click('[data-animation-toggle]');page('diagnostic-usage')
    assert js("return diagnosticLoads===0 && keptDiagnostic.contentDocument===keptDiagnosticDocument"),'Unrelated preferences reloaded diagnostic'
    click('.settings-back')
    assert js("return keptDiagnostic.isConnected && keptDiagnostic.contentDocument===keptDiagnosticDocument && document.querySelector('[data-client-view=settings]').hidden"),'Leaving settings destroyed retained diagnostic'
    click('.settings-link')
    assert js("return keptDiagnostic.contentDocument===keptDiagnosticDocument && !document.querySelector('[data-client-view=settings]').hidden"),'Reopening settings reloaded diagnostic'
    assert js("return !document.querySelector('[data-client-workspace] [data-client-view=settings]') && !document.querySelector('[data-tab-id=\"client:settings\"]')"),'Settings became a workspace tab'
    click('.settings-back')
    js("document.querySelector('[data-tab-id=\"client:chat\"]').closest('.workspace-group').querySelector('.workspace-transfer').click()")
    click('.settings-link')
    assert js("return keptDiagnostic.contentDocument===keptDiagnosticDocument && document.querySelector('[data-client-workspace]').hidden && document.querySelector('[data-client-workspace]').inert"),'Separate settings lost its iframe or left the split workspace active'
    click('.settings-back')
    assert js("return document.querySelectorAll('.workspace-group:not([hidden])').length===2 && !document.querySelector('[data-client-workspace]').hidden"),'Settings navigation lost the split workspace layout'
    click('[data-workspace-split]')
    click('.settings-link')
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
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=diagnostic-usage] iframe')?.contentDocument?.querySelector('.analysis-open-chat')"),'Return-to-chat action missing')
    js("window.returnFrame=document.querySelector('[data-module-page=diagnostic-usage] iframe');window.returnDocument=returnFrame.contentDocument;window.returnBoard=document.querySelector('.tab-workspace');returnDocument.querySelector('.analysis-open-chat').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea') && document.querySelector('[data-client-view=settings]').hidden"),'Embedded diagnostic did not return to parent chat')
    assert js("return returnFrame.isConnected && returnFrame.contentDocument===returnDocument && document.querySelector('.tab-workspace')===returnBoard"),'Same-session return reloaded client or Inspector'
    previous_session = js("return new URL(location.href).searchParams.get('session_dir')")
    js("window.hiddenDiagnostics=[...document.querySelectorAll('.diagnostic-frame')].map(frame=>({frame,src:frame.src,document:frame.contentDocument,loads:0}));window.diagnosticSourceObserver=new MutationObserver(records=>{for(const record of records)hiddenDiagnostics.find(item=>item.frame===record.target).loads++});for(const item of hiddenDiagnostics)diagnosticSourceObserver.observe(item.frame,{attributes:true,attributeFilter:['src']})")
    click('[aria-label="Новая сессия"]')
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')!=="+repr(previous_session)+" && document.querySelector('.connection-badge').classList.contains('completed')"),'Session did not change')
    assert js("return hiddenDiagnostics.length===4 && hiddenDiagnostics.every(item=>item.frame.isConnected && item.frame.src===item.src && item.frame.contentDocument===item.document && item.loads===0)"),'Changing chat reloaded a hidden diagnostic'
    click('.settings-link');page('diagnostic-usage')
    wait_for(lambda: js("return returnFrame.contentDocument!==returnDocument && !!returnFrame.contentDocument?.querySelector('#analysis-session')?.querySelector('option[value=\""+previous_session+"\"]')"),'Diagnostic did not reload its real session catalog')
    assert js("return hiddenDiagnostics.every(item=>item.frame===returnFrame ? item.loads===1 && new URL(item.frame.src).searchParams.get('session_dir')===new URL(location.href).searchParams.get('session_dir') : item.loads===0 && item.frame.src===item.src && item.frame.contentDocument===item.document)"),'Reveal did not refresh exactly one diagnostic to the latest session'
    js("diagnosticSourceObserver.disconnect()")
    js("const select=returnFrame.contentDocument.querySelector('#analysis-session');select.value="+repr(previous_session)+";select.dispatchEvent(new returnFrame.contentWindow.Event('change',{bubbles:true}));const url=new URL(location.href);url.searchParams.set('inspector',location.origin);history.replaceState(history.state,'',url)")
    wait_for(lambda: js("return new URL(returnFrame.contentDocument.querySelector('.analysis-open-chat').href).searchParams.get('session_dir')==="+repr(previous_session)), 'Inspector did not select the previous session')
    js("returnFrame.contentDocument.querySelector('.analysis-open-chat').click()")
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')==="+repr(previous_session)+" && !!document.querySelector('.connection-badge.completed') && !!document.querySelector('[data-client-view=chat]:not([hidden])')"),'Cross-session return did not select chat')
    assert js("return !new URL(location.href).searchParams.has('workspace_view') && new URL(location.href).searchParams.get('inspector')===location.origin"),'Cross-session return lost Inspector config or left its transient view parameter'
    print('PASS: installed diagnostic and selector use declared services; lazy mounting; drafts; disable/dispose; required management; persistent replacement; retained Inspector document across settings navigation and split workspace changes; all four Inspector pages; hidden session changes defer reload until reveal; same-session return identity and cross-session navigation',flush=True)
