"""User-facing preferences, real command dispatch and streaming scroll behavior."""
import base64
import json
from pathlib import Path


def run(command, js, wait_for, server):
    def click(selector):
        element = command('/element', {'using':'css selector','value':selector})
        command('/element/'+element['element-6066-11e4-a52e-4f735466cecf']+'/click', {})

    def settings(section):
        if not js("return !!document.querySelector('.settings-page')"):
            click('.settings-link')
        wait_for(lambda: js("return !!document.querySelector('[data-settings-section="+section+"]')"),'Settings navigation missing')
        click('[data-settings-section='+section+']')
        wait_for(lambda: js("return !!document.querySelector('[data-module-page="+section+"] input')"),'Module not mounted')

    def key(code, **modifiers):
        args = dict(code=code, key={'Escape':'Escape','Enter':'Enter'}.get(code,code), bubbles=True, cancelable=True, **modifiers)
        js('document.activeElement.dispatchEvent(new KeyboardEvent("keydown",'+json.dumps(args)+'))')

    def input_value(selector, value, event='input'):
        js('const e=document.querySelector('+json.dumps(selector)+');e.value='+json.dumps(str(value))+';e.dispatchEvent(new Event('+json.dumps(event)+',{bubbles:true}))')

    def back():
        click('.settings-back')
        wait_for(lambda: js("return !!document.querySelector('.composer textarea')"), 'Chat missing')

    # Values apply to the actual chat and existing drag width, and survive a reload.
    settings('appearance')
    js("window.normalNavHeight=document.querySelector('[data-settings-section=appearance]').getBoundingClientRect().height")
    input_value('[aria-label="Размер текста"]',20)
    input_value('[aria-label="Ширина диалога"]',960)
    assert js("return normalNavHeight===34 && !document.querySelector('[aria-label=\"Компактный интерфейс\"]')"), 'Compact layout is not the only layout'
    back()
    assert js("return getComputedStyle(document.querySelector('.message')).fontSize==='20px' && getComputedStyle(document.querySelector('.composer textarea')).fontSize==='20px'"), 'Font setting did not reach message/composer'
    assert js("return getComputedStyle(document.querySelector('.session-workspace')).getPropertyValue('--chat-max-width').trim()==='960px'"), 'Width setting did not reach chat'
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"), 'Reload failed')
    assert js("return getComputedStyle(document.querySelector('.composer textarea')).fontSize==='20px'"), 'Reload lost appearance'
    settings('appearance')
    # Storage rejection must leave the effective value and the slider in agreement.
    js("window.originalStorageSet=Storage.prototype.setItem;Storage.prototype.setItem=function(k,v){if(k==='proteus.fontSize')throw Error('fixture');return originalStorageSet.call(this,k,v)}")
    input_value('[aria-label="Размер текста"]',18)
    assert js("return document.querySelector('[aria-label=\"Размер текста\"]').value==='20' && document.querySelector('.settings-content').textContent.includes('хранилище недоступно')"), 'Appearance save failed silently'
    js("Storage.prototype.setItem=originalStorageSet")
    input_value('[aria-label="Размер текста"]',16)
    input_value('[aria-label="Ширина диалога"]',820)

    settings('shortcuts')
    wait_for(lambda: js("return !!document.querySelector('[data-bind=sidebar]')"),'Shortcut editor missing')
    input_value('[aria-label="Найти команду"]','список')
    assert js("return document.querySelectorAll('.shortcut-row').length===1"), 'Command search failed'
    input_value('[aria-label="Найти команду"]','')
    click('[data-bind=sidebar]')
    key('Comma',ctrlKey=True)
    assert js("return document.querySelector('.shortcut-settings [role=status]').textContent.includes('Уже назначено') && document.documentElement.dataset.shortcutRecording==='true'"), 'Conflict was accepted or recording leaked'
    key('KeyC',ctrlKey=True)
    assert js("return document.querySelector('.shortcut-settings [role=status]').textContent.includes('зарезервировано')"), 'Editing key was accepted'
    key('KeyJ',ctrlKey=True,altKey=True)
    assert js("return JSON.parse(localStorage.getItem('proteus.shortcuts')).sidebar==='Mod+Alt+KeyJ' && !document.documentElement.dataset.shortcutRecording"), 'Binding not saved'
    # Capture and cancellation must not trigger a command or retain capture after navigation.
    click('[data-bind=settings]')
    key('Escape')
    assert js("return !document.documentElement.dataset.shortcutRecording && !!document.querySelector('.settings-page')"), 'Escape recording cancellation failed'
    click('[data-bind=settings]')
    click('[data-settings-section=appearance]')
    assert js("return !document.documentElement.dataset.shortcutRecording"), 'Unmount leaked recording capture'
    settings('shortcuts')
    click('[data-clear=stop]')
    assert js("return JSON.parse(localStorage.getItem('proteus.shortcuts')).stop===null"), 'Disable shortcut failed'
    back()
    click('.composer-access-menu summary')
    key('Escape')
    assert js("return !document.querySelector('.composer-access-menu').open"), 'Disabling stop also disabled menu dismissal'
    settings('shortcuts')
    click('[data-reset=stop]')
    assert js("return JSON.parse(localStorage.getItem('proteus.shortcuts')).stop==='Escape'"), 'Reset shortcut failed'
    # Failure must retain the old binding, including after a reload.
    js("Storage.prototype.setItem=function(k,v){if(k==='proteus.shortcuts')throw Error('fixture');return originalStorageSet.call(this,k,v)}")
    click('[data-bind=sidebar]')
    key('KeyK',ctrlKey=True,altKey=True)
    assert js("return document.querySelector('.shortcut-settings [role=status]').textContent.includes('Не сохранено') && JSON.parse(localStorage.getItem('proteus.shortcuts')).sidebar==='Mod+Alt+KeyJ'"), 'Shortcut storage failure changed binding'
    key('Escape')
    js("Storage.prototype.setItem=originalStorageSet")
    Path('/tmp/proteus-settings-shortcuts.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    back()
    js("window.sidebarBefore=document.querySelector('.app-layout').classList.contains('sidebar-collapsed')")
    key('KeyB',ctrlKey=True)
    assert js("return document.querySelector('.app-layout').classList.contains('sidebar-collapsed')===sidebarBefore"), 'Old binding still active'
    key('KeyJ',ctrlKey=True,altKey=True)
    assert js("return document.querySelector('.app-layout').classList.contains('sidebar-collapsed')!==sidebarBefore"), 'New binding did not execute'
    key('KeyJ',ctrlKey=True,altKey=True)
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"),'Reload missing')
    js("window.sidebarBefore=document.querySelector('.app-layout').classList.contains('sidebar-collapsed')")
    key('KeyJ',ctrlKey=True,altKey=True)
    assert js("return document.querySelector('.app-layout').classList.contains('sidebar-collapsed')!==sidebarBefore"), 'Reload lost dispatch binding'
    key('KeyJ',ctrlKey=True,altKey=True)
    key('Comma',ctrlKey=True)
    wait_for(lambda: js("return !!document.querySelector('.settings-page')"),'Settings shortcut failed')
    settings('shortcuts')
    click('[data-reset-shortcuts]')
    assert js("return JSON.parse(localStorage.getItem('proteus.shortcuts')).sidebar==='Mod+KeyB'"), 'Reset all failed'

    settings('chat')
    input_value('[aria-label="Отправка сообщения"]','ctrl-enter','change')
    click('[aria-label="Автопрокрутка"]')
    back()
    area='.composer textarea'
    input_value(area,'Проверка настроек отправки')
    click(area)
    # Real Enter must insert a newline, with IME events never submitting.
    command('/actions',{'actions':[{'type':'key','id':'preferences-keys','actions':[{'type':'keyDown','value':'\ue007'},{'type':'keyUp','value':'\ue007'}]}]})
    assert js("return document.querySelector('.composer textarea').value.endsWith('\\n') && !document.querySelector('.composer-stop')"), 'Enter submitted in Ctrl+Enter mode'
    key('Enter',ctrlKey=True,isComposing=True)
    key('Enter',ctrlKey=True,repeat=True)
    assert js("return !document.querySelector('.composer-stop')"), 'IME/repeat submitted a prompt'
    before=server.model_requests
    server.stream_gate.clear()
    try:
        key('Enter',ctrlKey=True)
        wait_for(lambda: server.model_requests>before,'Ctrl+Enter did not submit')
        wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Абзац 3:')"),'Streaming prefix missing')
        js("window.results=document.querySelector('.results-panel');results.scrollTop=0;window.scrollBefore=results.scrollTop")
        server.stream_gate.set()
        wait_for(lambda: js("return !document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes('Абзац 31:')"),'Response not settled')
        assert js("return results.scrollTop===scrollBefore && !results.classList.contains('sticky-bottom')"), 'Disabled autoscroll moved the reading position'
        click('.jump-to-bottom')
        wait_for(lambda: js("return results.scrollHeight-results.scrollTop-results.clientHeight<5"),'Explicit jump failed with autoscroll disabled')
    finally:
        server.stream_gate.set()
    settings('chat')
    assert js("return document.querySelector('[aria-label=\"Отправка сообщения\"]').value==='ctrl-enter' && !document.querySelector('[aria-label=\"Автопрокрутка\"]').checked"), 'SPA lost chat settings'
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=chat]')"),'Settings reload failed')
    settings('chat')
    assert js("return document.querySelector('[aria-label=\"Отправка сообщения\"]').value==='ctrl-enter' && !document.querySelector('[aria-label=\"Автопрокрутка\"]').checked"), 'Reload lost chat settings'
    input_value('[aria-label="Отправка сообщения"]','enter','change')
    click('[aria-label="Автопрокрутка"]')
    back()
    input_value(area,'Проверка обычной отправки')
    click(area)
    key('Enter',ctrlKey=True)
    assert js("return document.querySelector('.composer textarea').value.includes('\\n') && !document.querySelector('.composer-stop')"),'Ctrl+Enter no longer inserts newline in Enter mode'
    key('Enter')
    wait_for(lambda: js("return !!document.querySelector('.composer-stop')"),'Enter did not submit')
    wait_for(lambda: js("return !document.querySelector('.composer-stop')"),'Enter response not settled')
    assert js("const r=document.querySelector('.results-panel');return r.classList.contains('sticky-bottom') && r.scrollHeight-r.scrollTop-r.clientHeight<5"),'Enabled autoscroll did not follow the response'
    settings('appearance')
    Path('/tmp/proteus-settings-appearance.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    back()
    print('PASS: appearance and density; shortcut capture/conflicts/remap/reset/cleanup; save rollback/reload; Enter modes and IME; streaming autoscroll off/on and explicit jump',flush=True)
