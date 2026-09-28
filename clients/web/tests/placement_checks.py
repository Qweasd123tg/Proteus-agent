"""Independent widget placement and content-sized hover/menu surfaces."""
import base64
import json
from pathlib import Path


def run(command, js, wait_for):
    def click(selector):
        js(f"document.querySelector({json.dumps(selector)}).click()")

    def settings():
        click('.settings-link')
        wait_for(lambda: js("return !!document.querySelector('.settings-page')"), 'Settings missing')
        click('[data-settings-section=extensions]')

    def place(id, value):
        click(f'[data-settings-id={id}]')
        js(f"const s=document.querySelector('[data-widget-placement={id}]');s.value={json.dumps(value)};s.dispatchEvent(new Event('change',{{bubbles:true}}))")

    wait_for(lambda: js("return !!document.querySelector('[data-widget-id=model-quota] span')?.shadowRoot?.querySelector('svg') && !!document.querySelector('[data-widget-id=context]')"), 'Live widgets missing')
    js("window.quotaWidget=document.querySelector('[data-widget-id=model-quota]');window.contextWidget=document.querySelector('[data-widget-id=context]');window.quotaPanel=document.querySelector('[data-extension-id=model-quota]');window.contextPanel=document.querySelector('[data-extension-id=context]')")
    settings()
    assert js("return !document.querySelector('.extension-settings > .extension-widget-placement')"), 'Global placement remains'
    place('model-quota','header')
    click('.settings-back')
    wait_for(lambda: js("return document.querySelector('[data-widget-slot=header] [data-widget-id=model-quota]')===quotaWidget && document.querySelector('[data-widget-slot=composer] [data-widget-id=context]')===contextWidget"), 'Moving one widget moved another')
    assert js("return document.querySelector('[data-extension-id=model-quota]')===quotaPanel"), 'Placement remounted extension'
    click('[data-widget-id=model-quota]')
    assert js("return document.querySelector('[data-tab-id=model-quota] [role=tab]').getAttribute('aria-selected')==='true'"), 'Moved widget lost interaction'
    settings()
    place('context','hidden')
    click('.settings-back')
    assert js("return !document.querySelector('[data-widget-id=context]') && document.querySelector('[data-extension-id=context]')===contextPanel && !!document.querySelector('[data-widget-id=model-quota]')"), 'Hiding disposed extension or hid others'
    settings()
    click('[data-settings-id=context]')
    assert js("return document.querySelector('[data-widget-placement=context]').value==='hidden'"), 'Reopening settings lost selection'
    js("window.placementSet=Storage.prototype.setItem;Storage.prototype.setItem=function(k,v){if(k==='proteus.ui.widget.context.position')throw new Error('storage fixture');return window.placementSet.call(this,k,v)}")
    place('context','header')
    assert js("return document.querySelector('[data-widget-placement=context]').value==='hidden' && document.querySelector('.extension-widget-placement [role=status]').textContent.includes('Не удалось')"), 'Failed write did not roll back'
    js("Storage.prototype.setItem=window.placementSet")
    place('context','composer')
    place('notes','header')
    click('.settings-back')
    assert js("return document.querySelector('[data-widget-id=context]')===contextWidget"), 'Showing widget recreated root'
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('[data-widget-slot=header] [data-widget-id=notes]') && !!document.querySelector('[data-widget-slot=header] [data-widget-id=model-quota]') && !!document.querySelector('[data-widget-slot=composer] [data-widget-id=context]')"), 'Independent placements lost on reload')

    def hover(selector):
        r=js(f"const r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]")
        command('/actions',{'actions':[{'type':'pointer','id':'hover-mouse','parameters':{'pointerType':'mouse'},'actions':[{'type':'pointerMove','duration':100,'origin':'viewport','x':round(r[0]),'y':round(r[1])}]}]})
        wait_for(lambda: js("return document.querySelector('.sidebar-hover').matches(':popover-open')"), 'Hover missing')
        wait_for(lambda: js("return !document.querySelector('.sidebar-hover').getAnimations().length"), 'Hover animation not settled')
        assert js("const p=document.querySelector('.sidebar-hover'),r=p.getBoundingClientRect();return r.width<=280 && r.height<=110 && r.bottom<innerHeight-8"), 'Hover stretched beyond content'

    hover('.session-item-shell')
    Path('/tmp/proteus-compact-chat-hover.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    hover('.sidebar-project')
    Path('/tmp/proteus-compact-project-hover.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    js("document.querySelector('.session-item-shell').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:130,clientY:230}))")
    assert js("return document.querySelector('.sidebar-menu').getBoundingClientRect().height<360"), 'Context menu stretched to bottom'
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    js("if(document.querySelector('.tab-workspace').hidden)document.querySelector('[data-workspace-toggle]').click()")
    click('.workspace-add')
    assert js("return document.querySelector('.workspace-picker').getBoundingClientRect().height<500"), 'Tab picker stretched to bottom'
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    print('PASS: independent per-extension placement; live roots, hide/show and persistence; write rollback; compact hover/context/tab menus',flush=True)
