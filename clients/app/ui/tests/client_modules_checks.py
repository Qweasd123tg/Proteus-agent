"""Actual registry installation, service wiring, teardown and embedded diagnostics."""
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin, loaded, capture=None):
    def click(selector):
        js('document.querySelector('+repr(selector)+').click()')
    def page(id):
        click('[data-settings-section='+id+']')
        wait_for(lambda: js('const page=document.querySelector("[data-module-page='+id+']");return !!page?.querySelector(".client-module-content")?.children.length || !!page?.querySelector("[data-select-slot]") || page?.querySelector("[data-builtin-module] input")?.checked===false'), 'Module missing: '+id)
    command('/url', {'url':web+'/?'+urlencode({'server':origin,'token':'extension-smoke'})})
    wait_for(loaded,'Client not connected')
    # A selection saved before an update removed a page it had turned off
    # stops at an explicit error, repaired from the chat or from settings.
    def break_selection():
        js("localStorage.setItem('proteus.ui.modules',JSON.stringify({disabled:['removed-page'],slots:{'composer-model':'model-selector','composer-access':'access-selector'}}))")
        command('/refresh', {})
        wait_for(loaded,'Client not connected after a broken selection')
        assert js("return !document.querySelector('.composer-model-menu')"),'A broken selection kept optional modules on'
    def repair(where):
        return "document.querySelector('[data-builtin-repair="+where+"]')"
    break_selection()
    wait_for(lambda: js("return !"+repair('workspace')+".hidden"),'Chat shows no repair action')
    assert 'removed-page' in js("return "+repair('workspace')+".previousElementSibling.textContent"), 'Error does not name the missing page'
    assert js("const a="+repair('workspace')+".parentElement.getBoundingClientRect(),b=document.querySelector('.composer-shell').getBoundingClientRect();return a.bottom<=b.top||a.top>=b.bottom||a.right<=b.left||a.left>=b.right"),'Error covers the composer'
    assert js("const b="+repair('workspace')+".getBoundingClientRect();return document.elementFromPoint(b.left+b.width/2,b.top+b.height/2)==="+repair('workspace')),'Repair action is not clickable'
    if capture:
        capture('broken-modules')
    js(repair('workspace')+".click()")
    wait_for(lambda: js("return !!document.querySelector('.composer-model-menu') && "+repair('workspace')+".hidden && !"+repair('workspace')+".previousElementSibling.textContent"),'Chat repair did not restore the selectors')
    break_selection()
    click('.settings-link')
    wait_for(lambda: js("return !"+repair('settings')+".hidden"),'Settings show no repair action')
    js(repair('settings')+".click()")
    wait_for(lambda: js("return "+repair('settings')+".hidden && "+repair('workspace')+".hidden"),'Settings repair left the error')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('.composer-model-menu')"),'Built-in selector not mounted')
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"),'Settings navigation missing')
    page('extensions')
    assert js("return !document.querySelector('.extension-management [data-builtin-module],.builtin-module-settings')"),'Built-in controls remain inside extension management'
    assert js("return document.querySelector('[data-extension-toggle=extensions]').checked && document.querySelector('[data-extension-toggle=extensions]').disabled"),'Required management has no visible locked switch'
    assert js("const nav=document.querySelector('.settings-nav');return ['appearance','chat','shortcuts','extensions','model-selector','access-selector'].every(id=>{const button=nav.querySelector('[data-settings-section=\"'+id+'\"]');let heading=button?.previousElementSibling;while(heading&&!heading.classList.contains('settings-nav-label'))heading=heading.previousElementSibling;return button?.parentElement===nav&&heading?.textContent==='Встроенные'})"),'Built-in entries do not form a separate sidebar group'
    assert js("const nav=document.querySelector('.settings-nav');return ['diagnostic-usage','diagnostic-analysis','diagnostic-architecture'].every(id=>{let heading=nav.querySelector('[data-settings-section=\"'+id+'\"]')?.previousElementSibling;while(heading&&!heading.classList.contains('settings-nav-label'))heading=heading.previousElementSibling;return heading?.textContent==='Расширения' && !!document.querySelector('[data-extension-choice=\"'+id+'\"] input:checked')})"),'Diagnostics are not enabled external packages in extension management'
    page('model-selector')
    click('[data-builtin-module=model-selector] input')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"),'Chat missing')
    assert js("return !document.querySelector('.composer-model-menu') && !!document.querySelector('.composer-access-menu')"),'Disabled module still mounted'
    click('.settings-link');page('model-selector')
    assert js("return document.querySelector('[data-settings-section=model-selector]').classList.contains('disabled')"),'Disabled builtin is not available for re-enabling'
    click('[data-builtin-module=model-selector] input')
    page('extensions')
    click('.extension-source > summary')
    js("document.querySelector('.extension-install input').value=location.origin+'/fixture/client/extension.json';document.querySelector('.extension-install').requestSubmit()")
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=client-test]')"),'Installed diagnostic absent from navigation')
    assert js("let heading=document.querySelector('[data-settings-section=client-test]').previousElementSibling;while(heading&&!heading.classList.contains('settings-nav-label'))heading=heading.previousElementSibling;return heading?.textContent==='Расширения' && ![...document.querySelectorAll('.settings-nav-label')].some(x=>x.textContent==='Диагностика')"),'Diagnostic extension has a separate navigation group'
    assert js("return !document.querySelector('[data-tab-id=client-test]') && !window.clientMounts"),'Settings-only module mounted in workspace'
    page('client-test')
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=client-test] [data-config-read]')"),'Declared agent service unavailable')
    js("document.querySelector('[data-custom-module=settings]').value='draft'")
    page('appearance');page('client-test')
    assert js("return clientMounts===1 && document.querySelector('[data-custom-module=settings]').value==='draft'"),'Navigation discarded custom page'
    wait_for(lambda: js("const image=document.querySelector('[data-extension-details=client-test] img');return image?.complete && image.naturalWidth>0"),'Package preview did not load')
    assert js("return document.querySelector('[data-extension-details=client-test] .extension-summary').textContent==='Browser fixture' && document.querySelector('[data-extension-toggle=client-test]').checked"),'Package page has no description or own switch'
    js("window.keptPreview=document.querySelector('[data-extension-details=client-test] img')")
    if capture:
        capture('extension-details')
    click('[data-extension-toggle=client-test]')
    wait_for(lambda: js("return clientAborts===1 && clientDisposals===1 && !document.querySelector('[data-settings-section=client-test]')"),'Page switch did not stop the package')
    assert js("return document.querySelector('.settings-page').dataset.settingsModule==='client-test' && keptPreview.isConnected && !document.querySelector('[data-extension-toggle=client-test]').checked && !document.querySelector('[data-module-page=client-test] [data-custom-module]')"),'Disabling lost package information or kept its running view'
    click('[data-extension-toggle=client-test]')
    wait_for(lambda: js("return clientMounts===2 && !!document.querySelector('[data-settings-section=client-test]')"),'Package could not be re-enabled on its page')
    page('extensions')
    click('[data-extension-choice=client-test] input')
    wait_for(lambda: js("return clientAborts===2 && clientDisposals===2 && !document.querySelector('[data-settings-section=client-test]')"),'Disable did not clean custom page')
    click('[data-settings-id=client-test]')
    assert js("return document.querySelector('[data-extension-details=client-test]') && !document.querySelector('[data-extension-toggle=client-test]').checked && clientMounts===2"),'Disabled package could not open its information without running code'
    command('/refresh', {})
    wait_for(loaded,'Client not connected after opening disabled package')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-details=client-test]')"),'Disabled package URL lost its information page')
    assert js("return !window.clientMounts && !document.querySelector('[data-extension-toggle=client-test]').checked"),'Reload executed the disabled package'
    click('[data-extension-toggle=client-test]')
    wait_for(lambda: js("return window.clientMounts===1 && !!document.querySelector('[data-settings-section=client-test]')"),'Reloaded package could not be enabled from its own page')
    click('[data-select-slot=composer-model][data-module-id=client-test]')
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('[data-custom-module=composer-model]')"),'Alternative selector not mounted')
    assert js("return !document.querySelector('.composer-model-menu')"),'Two implementations selected at once'
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('[data-client-slot=composer-model] [data-config-read] [data-custom-module=composer-model]')"),'Selection or declared service not restored')
    click('.settings-link');page('model-selector')
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
    for id in ['diagnostic-analysis','diagnostic-architecture']:
        page(id)
        wait_for(lambda: js('return !!document.querySelector("[data-module-page='+id+'] iframe")?.contentDocument?.querySelector(".inspector-shell")'), 'Diagnostic not loaded: '+id)
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=diagnostic-architecture] iframe').contentDocument.querySelector('[data-node-id=\"slot:workflow\"]')"),'Embedded architecture did not read real topology')
    js("window.removedArchitecture=document.querySelector('[data-module-page=diagnostic-architecture] iframe')")
    page('extensions');click('[data-extension-choice=diagnostic-architecture] .extension-actions button')
    assert js("return !removedArchitecture.isConnected && !document.querySelector('[data-settings-section=diagnostic-architecture]') && !!document.querySelector('[data-extension-available=diagnostic-architecture]')"),'Diagnostic package could not be removed'
    click('[data-extension-available=diagnostic-architecture]')
    page('diagnostic-architecture')
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=diagnostic-architecture] iframe')?.contentDocument?.querySelector('.inspector-shell')"),'Diagnostic package could not be added again')
    page('extensions');click('[data-extension-choice=diagnostic-usage] input')
    assert js("return !keptDiagnostic.isConnected && !document.querySelector('[data-settings-section=diagnostic-usage]')"),'Disabled diagnostic retained iframe or navigation entry'
    command('/refresh', {})
    wait_for(loaded,'Client not connected after disabling diagnostic')
    click('.settings-link');page('extensions')
    assert js("return !document.querySelector('[data-settings-section=diagnostic-usage]') && !document.querySelector('[data-extension-choice=diagnostic-usage] input').checked"),'Disabled external diagnostic was restored on restart'
    click('[data-extension-choice=diagnostic-usage] input')
    for id in ['diagnostic-analysis','diagnostic-architecture']:
        page(id)
        wait_for(lambda: js('return !!document.querySelector("[data-module-page='+id+'] iframe")?.contentDocument?.querySelector(".inspector-shell")'), 'Diagnostic not loaded after restart: '+id)
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
    assert js("return hiddenDiagnostics.length===3 && hiddenDiagnostics.every(item=>item.frame.isConnected && item.frame.src===item.src && item.frame.contentDocument===item.document && item.loads===0)"),'Changing chat reloaded a hidden diagnostic'
    click('.settings-link');page('diagnostic-usage')
    wait_for(lambda: js("return returnFrame.contentDocument!==returnDocument && !!returnFrame.contentDocument?.querySelector('#analysis-session')?.querySelector('option[value=\""+previous_session+"\"]')"),'Diagnostic did not reload its real session catalog')
    assert js("return hiddenDiagnostics.every(item=>item.frame===returnFrame ? item.loads===1 && new URL(item.frame.src).searchParams.get('session_dir')===new URL(location.href).searchParams.get('session_dir') : item.loads===0 && item.frame.src===item.src && item.frame.contentDocument===item.document)"),'Reveal did not refresh exactly one diagnostic to the latest session'
    js("diagnosticSourceObserver.disconnect()")
    js("const select=returnFrame.contentDocument.querySelector('#analysis-session');select.value="+repr(previous_session)+";select.dispatchEvent(new returnFrame.contentWindow.Event('change',{bubbles:true}))")
    wait_for(lambda: js("return new URL(returnFrame.contentDocument.querySelector('.analysis-open-chat').href).searchParams.get('session_dir')==="+repr(previous_session)), 'Inspector did not select the previous session')
    js("returnFrame.contentDocument.querySelector('.analysis-open-chat').click()")
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')==="+repr(previous_session)+" && !!document.querySelector('.connection-badge.completed') && !!document.querySelector('[data-client-view=chat]:not([hidden])')"),'Cross-session return did not select chat')
    assert js("return !new URL(location.href).searchParams.has('workspace_view')"),'Cross-session return left its transient view parameter'
    print('PASS: diagnostics are external managed packages; remove/add and disable/restart; broken saved selection repaired beside its error; installed diagnostic and selector use declared services; lazy mounting; drafts; disable/dispose; required management; persistent replacement; retained Inspector document across settings navigation and split workspace changes; all three Inspector pages; hidden session changes defer reload until reveal; same-session return identity and cross-session navigation',flush=True)
