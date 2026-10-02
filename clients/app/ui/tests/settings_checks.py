"""Shared navigation geometry, extension settings panes and real pointer reorder."""
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

    js("window.chatWidth=document.querySelector('.sidebar').getBoundingClientRect().width;const row=document.querySelector('.session-item');window.chatRow={height:row.getBoundingClientRect().height,font:getComputedStyle(row.querySelector('.session-id')).fontSize,radius:getComputedStyle(row).borderRadius}")
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"), 'Settings missing')
    click('[data-settings-section=extensions]')
    assert js("const row=document.querySelector('[data-settings-section=extensions]');return row.getBoundingClientRect().height===chatRow.height && getComputedStyle(row).fontSize===chatRow.font && getComputedStyle(row).borderRadius===chatRow.radius"), 'Settings/chat navigation geometry differs'
    assert js("return !document.querySelector('[aria-label^=\"Выше:\"], [aria-label^=\"Ниже:\"]')"), 'Order arrows remain'
    wait_for(lambda: len(order())>2,'Module list missing')
    before = order()
    # Move the third row to the top using trusted pointer events, inspect before drop.
    handle = f'[data-reorder="{before[2]}"]'
    js(f"document.querySelector('{handle}').scrollIntoView({{block:'center'}})")
    start = js(f"const r=document.querySelector('{handle}').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]")
    target = js("const r=document.querySelector('.extension-list').firstElementChild.getBoundingClientRect();return [r.x+20,r.y+4]")
    js("window.orderSaved=localStorage.getItem('proteus.ui.extensions')")
    pointer([move(*start), {'type': 'pointerDown', 'button': 0}, move(*target)])
    assert js("return !!document.querySelector('.extension-drag-ghost') && localStorage.getItem('proteus.ui.extensions')===orderSaved"), 'Drag preview missing or saved before drop: ' + str(js("return {ghost:!!document.querySelector('.extension-drag-ghost'),same:localStorage.getItem('proteus.ui.extensions')===orderSaved,placeholder:!!document.querySelector('.drag-placeholder')}"))
    pointer([{'type': 'pointerUp', 'button': 0}])
    expected = [before[2], before[0], before[1], *before[3:]]
    assert order() == expected, 'Drop swapped rows instead of inserting'
    assert js("return JSON.parse(localStorage.getItem('proteus.ui.extensions')).panels.map(x=>x.id)") == expected, 'Order not persisted'
    # Cancel a second pointer drag without a write or global Escape propagation.
    start = js(f"const r=document.querySelector('{handle}').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]")
    target = js("const r=document.querySelector('.extension-list').children[2].getBoundingClientRect();return [r.x+20,r.bottom-4]")
    js("window.escapedToApp=0;window.watchEscape=e=>{if(e.key==='Escape')escapedToApp++};window.addEventListener('keydown',watchEscape);window.orderSaved=localStorage.getItem('proteus.ui.extensions')")
    pointer([move(*start), {'type': 'pointerDown', 'button': 0}, move(*target)])
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    pointer([{'type': 'pointerUp', 'button': 0}])
    assert order() == expected and js("return !document.querySelector('.extension-drag-ghost') && escapedToApp===0 && localStorage.getItem('proteus.ui.extensions')===orderSaved"), 'Escape did not cancel drag cleanly'
    click('[data-settings-id=usage]')
    wait_for(lambda: js("return !!document.querySelector('.extension-options-content')?.shadowRoot?.querySelector('form')"), 'Settings form not mounted')
    wait_for(lambda: js("return !document.querySelector('.extension-options').getAnimations().some(a=>a.playState==='running')"), 'Options animation did not settle')
    assert js("const n=document.querySelector('.settings-nav').getBoundingClientRect(),l=document.querySelector('.settings-content').getBoundingClientRect(),r=document.querySelector('.extension-options').getBoundingClientRect();return n.right<=l.left && r.left===l.left && r.right<=l.right+1 && r.width>300"), 'Module details escaped the Settings screen'
    js("window.keptOptions=document.querySelector('.extension-options-content');keptOptions.shadowRoot.querySelector('[name=model]').value='draft-model'")
    # A dropdown consumes Escape first, then the pane consumes the next one.
    js("keptOptions.shadowRoot.querySelector('select').click()")
    assert js("return !!keptOptions.shadowRoot.querySelector('.select-picker')"), 'Settings dropdown not open'
    command('/actions', {'actions': [{'type': 'key', 'id': 'settings-key', 'actions': [{'type': 'keyDown', 'value': '\ue00c'}, {'type': 'keyUp', 'value': '\ue00c'}]}]})
    assert js("return !keptOptions.shadowRoot.querySelector('.select-picker')?.matches(':popover-open') && !document.querySelector('.extension-options').hidden && escapedToApp===0"), 'Dropdown Escape closed pane or reached agent'
    wait_for(lambda: js("return !keptOptions.shadowRoot.querySelector('.select-picker')"), 'Closed dropdown did not finish leaving')
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    assert js("return document.querySelector('.extension-options').hidden && document.activeElement.dataset.settingsId==='usage' && escapedToApp===0"), 'Panel Escape did not restore focus or reached agent'
    click('[data-settings-id=usage]')
    click('[data-settings-section=diagnostic-usage]')
    wait_for(lambda: js("return document.querySelector('.extension-options').getBoundingClientRect().width===0"), 'Options leaked into another section')
    click('[data-settings-section=extensions]')
    assert js("return document.querySelector('.extension-options-content')===keptOptions && keptOptions.shadowRoot.querySelector('[name=model]').value==='draft-model'"), 'Hiding settings lost unsaved form'
    wait_for(lambda: js("return !document.querySelector('.settings-nav').getAnimations({subtree:true}).some(a=>a.playState==='running')"), 'Navigation transition did not settle')
    Path('/tmp/proteus-settings-three-panes.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    for width in [900, 620, 390]:
        command('/window/rect', {'width': width, 'height': 1000})
        assert js("const n=document.querySelector('.settings-nav').getBoundingClientRect(),l=document.querySelector('.settings-content').getBoundingClientRect(),r=document.querySelector('.extension-options').getBoundingClientRect();return n.right<=l.left && r.left===l.left && r.width>0 && r.right<=l.right+1 && getComputedStyle(document.querySelector('.settings-nav')).flexDirection==='column'"), 'Settings switched to mobile layout'
    command('/window/rect', {'width': 1440, 'height': 1000})
    click('.settings-back')
    wait_for(lambda: js("return !!document.querySelector('.settings-link') && document.querySelector('[data-client-view=settings]').hidden"), 'Settings cleanup failed')
    js("window.removeEventListener('keydown',watchEscape)")
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('.settings-link')"), 'Reload failed')
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=extensions]')"),'Settings missing')
    click('[data-settings-section=extensions]')
    wait_for(lambda: js("return document.querySelectorAll('.extension-list [data-extension-choice]').length>2"), 'Extensions missing after reload')
    assert order() == expected, 'Reload lost drag order'
    click('.settings-back')
    print('PASS: shared chat/settings geometry; pointer insertion and cancel; persisted order; module detail page; dropdown/pane Escape isolation; drafts; desktop panes at every window width', flush=True)
