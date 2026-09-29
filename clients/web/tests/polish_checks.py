"""User preferences across live panes, new/resumed chats and reloads."""
import json


def run(command, js, wait_for):
    def click(selector):
        js(f"document.querySelector({json.dumps(selector)}).click()")

    def menu(label):
        js(f"[...document.querySelectorAll('.sidebar-menu button')].find(b=>b.textContent==={json.dumps(label)}).click()")

    def settings(section):
        click('.settings-link')
        wait_for(lambda: js("return !!document.querySelector('[data-settings-section=appearance]')"), 'Settings absent')
        click(f'[data-settings-section={section}]')
        wait_for(lambda: js('return !!document.querySelector("[data-module-page='+section+'] input")'), 'Settings module pending')

    # One live compact instance follows its placement, including across SPA mounts.
    wait_for(lambda: js("return !!document.querySelector('[data-widget-id=model-quota] span')?.shadowRoot?.querySelector('svg')"), 'Compact quota did not render')
    js("window.savedWidget=document.querySelector('[data-widget-id=model-quota]');window.savedPanel=document.querySelector('[data-extension-id=model-quota]');savedWidget.click()")
    assert js("return document.querySelector('[data-tab-id=model-quota] [role=tab]').getAttribute('aria-selected')==='true'"), 'Widget did not open its tab'
    settings('extensions')
    click('[data-settings-id=model-quota]')
    js("const s=document.querySelector('[data-widget-placement=model-quota]');s.value='header';s.dispatchEvent(new Event('change',{bubbles:true}))")
    click('.settings-back')
    wait_for(lambda: js("return document.querySelector('[data-widget-slot=header] [data-widget-id=model-quota]')===savedWidget"), 'Widget not moved to header')
    assert js("return document.querySelector('[data-extension-id=model-quota]')===savedPanel"), 'Moving widget remounted panel'
    settings('extensions')
    js("window.switchNode=document.querySelector('[data-extension-choice=model-quota] input');switchNode.click()")
    assert js("return document.querySelector('[data-extension-choice=model-quota] input')===switchNode && !switchNode.checked"), 'Extension toggle was replaced'
    js("switchNode.click()")
    click('.settings-back')
    settings('appearance')
    assert js("return parseFloat(getComputedStyle(document.querySelector('.settings-toggle'),'::before').transitionDuration)>0"), 'Toggle has no motion'
    click('[data-animation-toggle]')
    wait_for(lambda: js("return document.documentElement.dataset.animations==='off' && localStorage.getItem('proteus.animations')==='false'"), 'Motion setting not applied/persisted')
    assert js("return getComputedStyle(document.querySelector('.settings-toggle'),'::before').transitionDuration==='0s'"), 'Switch still animates while disabled'
    click('.settings-back')
    click('[data-panel-toggle=sidebar]')
    click('[data-panel-toggle=sidebar]')
    assert js("return getComputedStyle(document.querySelector('.sidebar-surface')).animationName==='none'"), 'Panel still animates while disabled'
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('.session-item-shell')"), 'Reload absent')
    wait_for(lambda: js("return document.documentElement.dataset.animations==='off' && !!document.querySelector('[data-widget-slot=header] .extension-widget')"), 'Reload lost UI preferences')
    settings('appearance')
    click('[data-animation-toggle]')
    click('.settings-back')

    assert js("return [...document.querySelectorAll('[data-delete-session]')].every(b=>b.getBoundingClientRect().height===0)"), 'Delete action leaked into sidebar'
    # Real context menu -> local pin/name/archive, retaining the loaded chat.
    js("window.rowId=document.querySelector('.session-item-shell').dataset.sessionDir;document.querySelector('.session-item-shell [data-sidebar-menu]').click()")
    menu('Закрепить')
    assert js("return document.querySelector('.session-item-shell').dataset.pinned==='true'"), 'Pin not applied'
    click('.session-item-shell [data-sidebar-menu]')
    menu('Переименовать')
    js("const f=document.querySelector('.sidebar-rename');f.querySelector('input').value='Мой чат';f.requestSubmit()")
    wait_for(lambda: js("return document.querySelector('.session-item-shell').dataset.hoverTitle==='Мой чат'"), 'Rename not applied')
    click('.session-item-shell [data-sidebar-menu]')
    menu('Архивировать')
    assert js("return ![...document.querySelectorAll('.session-item-shell')].some(r=>r.dataset.sessionDir===rowId)"), 'Archived session remains in current list'
    click('.sidebar-project [data-sidebar-menu]')
    menu('Показать архив чатов')
    wait_for(lambda: js("return document.querySelector('.session-item-shell')?.dataset.archived==='true'"), 'Archive not accessible')
    click('.session-item-shell [data-sidebar-menu]')
    menu('Вернуть из архива')
    click('.sidebar-project [data-sidebar-menu]')
    menu('Показать текущие чаты')
    command('/refresh', {})
    wait_for(lambda: js("return document.querySelector('.session-item-shell')?.dataset.hoverTitle==='Мой чат'"), 'Reload lost session label')
    assert js("return document.querySelector('.session-item-shell').dataset.pinned==='true'"), 'Reload lost pin'
    print('PASS: live widgets move without remount; motion off/on and reload; context menu pin/rename/archive persistence', flush=True)

    # Last successful manual pair is used only for newly created chats.
    js("sessionStorage.setItem('polish.originalSession',new URL(location.href).searchParams.get('session_dir'))")
    click('.composer-model-menu summary')
    wait_for(lambda: js("return [...document.querySelectorAll('.composer-model-menu .menu-option-title')].some(x=>x.textContent==='Fixture 2')"), 'Second fixture model absent')
    js("[...document.querySelectorAll('.composer-model-menu .menu-option-row')].find(b=>b.textContent.includes('Fixture 2')).click()")
    wait_for(lambda: js("return JSON.parse(localStorage.getItem('proteus.model.last-selection')||'null')?.model==='fixture-model-2'"), 'Manual model not remembered')
    js("[...document.querySelectorAll('.composer-model-menu .menu-option')].find(b=>b.querySelector('.menu-option-title')?.textContent==='high').click()")
    wait_for(lambda: js("return JSON.parse(localStorage.getItem('proteus.model.last-selection')||'null')?.effort==='high'"), 'Effort not remembered')
    command('/refresh', {})
    wait_for(lambda: js("return document.querySelector('.connection-badge')?.classList.contains('completed')"), 'Model reload not connected')
    click('[aria-label="Новая сессия"]')
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')!==sessionStorage.getItem('polish.originalSession') && document.querySelector('.connection-badge')?.classList.contains('completed') && document.querySelector('.composer-model-menu summary')?.dataset.uiTooltip==='fixture-model-2'"), 'New chat lost selected model')
    assert js("return document.querySelector('.composer-menu-meta')?.textContent.includes('High') || [...document.querySelectorAll('.composer-model-menu .menu-option.active')].some(b=>b.querySelector('.menu-option-title')?.textContent==='high')"), 'New chat lost effort'
    click('.composer-model-menu summary')
    js("[...document.querySelectorAll('.composer-model-menu .menu-option-row')].find(b=>b.querySelector('.menu-option-title').textContent==='Fixture').click()")
    wait_for(lambda: js("return JSON.parse(localStorage.getItem('proteus.model.last-selection')||'null')?.model==='fixture-model'"), 'Second model selection did not save')
    js("[...document.querySelectorAll('.session-item-shell')].find(r=>r.dataset.sessionDir===sessionStorage.getItem('polish.originalSession')).querySelector('.session-item').click()")
    wait_for(lambda: js("return document.querySelector('.composer-model-menu summary')?.dataset.uiTooltip==='fixture-model-2' && document.querySelector('.connection-badge')?.classList.contains('completed')"), 'Resume overwrote existing session model')
    assert js("return JSON.parse(localStorage.getItem('proteus.model.last-selection')).model==='fixture-model'"), 'Resume replaced last manual preference'
    print('PASS: last model+effort persisted; new chat restores pair; resumed chat retains own settings', flush=True)

    check_restore_failure(command,js,wait_for)


def check_restore_failure(command,js,wait_for):
    # Failure must remain visible after the new session's SSE snapshot replaces history.
    js("window.restoreFetch=window.fetch;window.failedRestore=false;window.fetch=async(input,...args)=>{const path=new URL(input.url||input,location.href).pathname;if(path==='/model'&&!window.failedRestore){window.failedRestore=true;return new Response('model unavailable fixture',{status:503})}return restoreFetch(input,...args)};window.beforeFailedRestore=new URL(location.href).searchParams.get('session_dir')")
    js("document.querySelector('[aria-label=\"Новая сессия\"]').click()")
    try:
        wait_for(lambda: js("return window.failedRestore && new URL(location.href).searchParams.get('session_dir')!==beforeFailedRestore && document.querySelector('.connection-badge')?.classList.contains('completed')"), 'Failed preference blocked chat connection')
    except AssertionError:
        print(js("return {failedRestore,url:location.href,connection:document.querySelector('.connection-badge')?.outerHTML,toasts:document.querySelector('.toast-stack')?.textContent,sidebar:document.querySelector('.sidebar')?.textContent}"),flush=True)
        raise
    assert js("return document.querySelector('.toast-stack').textContent.includes('Сохранённая модель или effort недоступны')"), 'Session snapshot erased restore error'
    js("window.fetch=restoreFetch;document.querySelector('.toast button').click()")
    print('PASS: failed model restore keeps chat usable and visible dismissible error', flush=True)
