"""Extension management, ordinary settings pages and real pointer reorder."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    def click(selector):
        element = command('/element', {'using': 'css selector', 'value': selector})
        command('/element/' + element['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    def pointer(actions):
        command('/actions', {'actions': [{'type': 'pointer', 'id': 'settings-mouse', 'parameters': {'pointerType': 'mouse'}, 'actions': actions}]})

    def move(x, y):
        return {'type': 'pointerMove', 'duration': 100, 'origin': 'viewport', 'x': round(x), 'y': round(y)}

    def order():
        return js("return [...document.querySelectorAll('.extension-list [data-extension-choice]')].map(x=>x.dataset.extensionChoice)")

    js("const row=document.querySelector('.session-item');window.chatRow={height:row.getBoundingClientRect().height,font:getComputedStyle(row.querySelector('.session-id')).fontSize,radius:getComputedStyle(row).borderRadius}")
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"), 'Settings missing')
    click('[data-settings-section=extensions]')
    wait_for(lambda: len(order()) > 2, 'Extension management missing')
    js("document.querySelector('[data-extension-available=diagnostic-usage]')?.click()")
    assert js("const nav=document.querySelector('.settings-nav'),manager=document.querySelector('.extension-management');return manager.closest('.settings-content') && !nav.querySelector('.extension-list,input,select,[data-reorder],.extension-install') && !document.querySelector('.extension-settings-sidebar,.extension-options,.settings-extensions-nav')"), 'Management controls leaked into navigation'
    assert js("return !document.querySelector('[data-module-page=extensions] [data-builtin-module],.builtin-module-settings')"), 'Built-ins remain inside extension management'
    assert js("const nav=document.querySelector('.settings-nav');return [...document.querySelectorAll('[data-extension-choice]')].every(row=>{const expected=row.querySelector('input').checked&&!row.querySelector('[data-settings-id]').disabled,button=nav.querySelector('[data-settings-section=\"'+CSS.escape(row.dataset.extensionChoice)+'\"]');return !!button===expected && (!button||button.parentElement===nav)})"), 'Navigation includes a disabled or broken package, or misses an enabled one'
    assert js("const a=document.querySelector('[data-settings-section=appearance]'),b=document.querySelector('[data-settings-section=usage]');return b.getBoundingClientRect().height===chatRow.height && getComputedStyle(b).fontSize===getComputedStyle(a).fontSize && getComputedStyle(b).borderRadius===chatRow.radius"), 'Extension entry differs from ordinary settings'
    assert js("return !document.querySelector('[data-module-page=usage] .extension-view-content') && document.querySelector('[data-builtin-repair=settings]').getBoundingClientRect().width===0"), 'Management executed settings or exposed a hidden repair action'
    before = order()
    handle = f'[data-reorder="{before[2]}"]'
    js(f"document.querySelector('{handle}').scrollIntoView({{block:'center'}})")
    start = js(f"const r=document.querySelector('{handle}').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]")
    target = js("const r=document.querySelector('.extension-list').firstElementChild.getBoundingClientRect();return [r.x+20,r.y+4]")
    js("window.orderSaved=localStorage.getItem('proteus.ui.extensions')")
    pointer([move(*start), {'type': 'pointerDown', 'button': 0}, move(*target)])
    assert js("return !!document.querySelector('.extension-drag-ghost') && localStorage.getItem('proteus.ui.extensions')===orderSaved"), 'Drag preview missing or saved before drop'
    pointer([{'type': 'pointerUp', 'button': 0}])
    expected = [before[2], before[0], before[1], *before[3:]]
    assert order() == expected, 'Drop did not insert the row'
    assert js("return JSON.parse(localStorage.getItem('proteus.ui.extensions')).panels.map(x=>x.id)") == expected, 'Order not persisted'
    start = js(f"const r=document.querySelector('{handle}').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]")
    target = js("const r=document.querySelector('.extension-list').children[2].getBoundingClientRect();return [r.x+20,r.bottom-4]")
    js("window.escapedToApp=0;window.watchEscape=e=>{if(e.key==='Escape')escapedToApp++};window.addEventListener('keydown',watchEscape);window.orderSaved=localStorage.getItem('proteus.ui.extensions')")
    pointer([move(*start), {'type': 'pointerDown', 'button': 0}, move(*target)])
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    pointer([{'type': 'pointerUp', 'button': 0}])
    assert order() == expected and js("return !document.querySelector('.extension-drag-ghost') && escapedToApp===0 && localStorage.getItem('proteus.ui.extensions')===orderSaved"), 'Escape did not cancel drag'
    click('[data-settings-section=usage]')
    wait_for(lambda: js("return !!document.querySelector('[data-module-page=usage] .extension-view-content')?.shadowRoot?.querySelector('form')"), 'Ordinary extension page did not mount')
    js("window.keptOptions=document.querySelector('[data-module-page=usage] .extension-view-content');keptOptions.shadowRoot.querySelector('[name=model]').value='draft-model'")
    js("keptOptions.shadowRoot.querySelector('select').click()")
    command('/actions', {'actions': [{'type': 'key', 'id': 'settings-key', 'actions': [{'type': 'keyDown', 'value': '\ue00c'}, {'type': 'keyUp', 'value': '\ue00c'}]}]})
    assert js("return !keptOptions.shadowRoot.querySelector('.select-picker')?.matches(':popover-open') && document.querySelector('.settings-page').dataset.settingsModule==='usage' && escapedToApp===0"), 'Dropdown Escape reached the settings host or agent'
    wait_for(lambda: js("return !keptOptions.shadowRoot.querySelector('.select-picker')"), 'Dropdown did not finish closing')
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    assert js("return document.querySelector('.settings-page').dataset.settingsModule==='extensions' && document.activeElement.dataset.settingsSection==='extensions' && escapedToApp===0"), 'Page Escape did not return to management'
    click('[data-settings-section=usage]')
    click('[data-settings-section=notes]')
    click('[data-settings-section=diagnostic-usage]')
    wait_for(lambda: js("return document.querySelector('[data-module-page=usage]').getBoundingClientRect().width===0"), 'Extension parameters leaked into another page')
    click('[data-settings-section=usage]')
    assert js("return document.querySelector('[data-module-page=usage] .extension-view-content')===keptOptions && keptOptions.shadowRoot.querySelector('[name=model]').value==='draft-model'"), 'Another extension or section discarded the draft'
    wait_for(lambda: js("return !document.querySelector('.settings-page').getAnimations({subtree:true}).some(a=>a.playState==='running')"), 'Settings transition did not settle')
    Path('/tmp/proteus-settings-active-extensions.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    click('[data-settings-section=extensions]')
    click('[data-extension-choice=usage] input')
    wait_for(lambda: js("return !document.querySelector('[data-settings-section=usage]') && !keptOptions.isConnected"), 'Disabling kept a navigation item or settings runtime')
    click('[data-extension-choice=usage] input')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=usage]')"), 'Enabling did not add a navigation item')
    assert js("return !document.querySelector('[data-module-page=usage]')"), 'Enabling eagerly executed the settings entry'
    click('[data-settings-section=usage]')
    for width in [900, 620, 390]:
        command('/window/rect', {'width': width, 'height': 1000})
        assert js("const nav=document.querySelector('.settings-nav'),n=nav.getBoundingClientRect(),c=document.querySelector('.settings-content').getBoundingClientRect();return Math.abs(n.right-c.left)<=1 && c.width>=320 && getComputedStyle(nav.parentElement).gridTemplateColumns.split(' ').length===2"), 'Settings gained an extra column or collapsed the parameters'
    command('/window/rect', {'width': 1440, 'height': 1000})
    click('.settings-back')
    js("window.removeEventListener('keydown',watchEscape);const config=JSON.parse(localStorage.getItem('proteus.ui.extensions'));config.panels.push({id:'broken-settings',url:location.origin+'/fixture/external/extension.json',enabled:true,collapsed:false});localStorage.setItem('proteus.ui.extensions',JSON.stringify(config))")
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('.settings-link')"), 'Reload failed')
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"), 'Settings missing after reload')
    click('[data-settings-section=extensions]')
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=broken-settings] .extension-error')"), 'Broken manifest was not reported in management')
    assert js("return !document.querySelector('[data-settings-section=broken-settings]')"), 'Broken manifest appeared as a settings page'
    js("document.querySelector('[data-extension-choice=broken-settings] .extension-actions button').click()")
    assert order() == expected, 'Reload lost drag order'
    click('.settings-back')
    print('PASS: separate extension manager; only enabled loaded packages in ordinary navigation; lazy per-package settings; disable teardown; pointer reorder/cancel; persisted order; dropdown/page Escape; retained drafts; two columns', flush=True)
