"""Independent widget placement and content-sized hover/menu surfaces."""
import base64
import json
from pathlib import Path
from sidebar_title_checks import run as check_sidebar_titles


def run(command, js, wait_for):
    check_sidebar_titles(command, js, wait_for)

    def click(selector):
        js(f"document.querySelector({json.dumps(selector)}).click()")

    def settings():
        click('.settings-link')
        wait_for(lambda: js("return !document.querySelector('[data-client-view=settings]').hidden"), 'Settings missing')
        click('[data-settings-section=extensions]')

    def place(id, value):
        click(f'[data-settings-section={id}]')
        js(f"const s=document.querySelector('[data-widget-placement={id}]');s.value={json.dumps(value)};s.dispatchEvent(new Event('change',{{bubbles:true}}))")

    wait_for(lambda: js("return !!document.querySelector('[data-widget-id=model-quota] span')?.shadowRoot?.querySelector('svg') && !!document.querySelector('[data-widget-id=context]')"), 'Live widgets missing')
    wait_for(lambda: js("return !!document.querySelector('[data-widget-slot=header] [data-widget-id=agent-info]')"), 'Catalog default did not start the agent widget in the header')
    assert js("return localStorage.getItem('proteus.ui.widget.agent-info.position')===null"), 'Catalog default was written as a user choice'
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
    click('[data-settings-section=context]')
    assert js("return document.querySelector('[data-widget-placement=context]').value==='hidden'"), 'Reopening settings lost selection'
    js("window.placementSet=Storage.prototype.setItem;Storage.prototype.setItem=function(k,v){if(k==='proteus.ui.widget.context.position')throw new Error('storage fixture');return window.placementSet.call(this,k,v)}")
    place('context','header')
    assert js("return document.querySelector('[data-widget-placement=context]').value==='hidden' && document.querySelector('[data-module-page=context] .extension-widget-placement [role=status]').textContent.includes('Не удалось')"), 'Failed write did not roll back'
    js("Storage.prototype.setItem=window.placementSet")
    place('context','composer')
    place('notes','header')
    click('.settings-back')
    assert js("return document.querySelector('[data-widget-id=context]')===contextWidget"), 'Showing widget recreated root'
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('[data-widget-slot=header] [data-widget-id=notes]') && !!document.querySelector('[data-widget-slot=header] [data-widget-id=model-quota]') && !!document.querySelector('[data-widget-slot=composer] [data-widget-id=context]')"), 'Independent placements lost on reload')
    # Returning from Settings restores the selected quota tab; composer widgets
    # belong to chat, so select that surface before sending pointer gestures.
    click('.brand')
    wait_for(lambda: js("return !document.querySelector('[data-client-view=chat]').hidden"), 'Chat did not reveal composer widgets')

    # Pointer drag must move the actual live root across the two host surfaces.
    def point(selector):
        return js(f"const r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return [Math.round(r.x+r.width/2),Math.round(r.y+r.height/2)]")

    def drag_widget(id, destination, cancel=False):
        js(f"document.querySelector('[data-widget-id={id}]').scrollIntoView({{inline:'center',block:'nearest'}})")
        wait_for(lambda: js(f"const widget=document.querySelector('[data-widget-id={id}]'),r=widget.getBoundingClientRect();return r.width>0 && document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest('[data-widget-id]')===widget"), 'Widget is not ready for a pointer drag')
        start=point(f'[data-widget-id={id}]')
        command('/actions',{'actions':[{'type':'pointer','id':'widget-drag','parameters':{'pointerType':'mouse'},'actions':[{'type':'pointerMove','origin':'viewport','x':start[0],'y':start[1]},{'type':'pointerDown','button':0},{'type':'pointerMove','duration':100,'origin':'viewport','x':start[0]+8,'y':start[1]}]}]})
        assert js("return document.querySelector('[data-widget-slot=header]').classList.contains('widget-drop-target')"), 'Empty header drop target missing'
        end=point(f'[data-widget-slot={destination}] .extension-widgets')
        command('/actions',{'actions':[{'type':'pointer','id':'widget-drag','parameters':{'pointerType':'mouse'},'actions':[{'type':'pointerMove','duration':160,'origin':'viewport','x':end[0],'y':end[1]}]}]})
        if cancel:
            js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
        command('/actions',{'actions':[{'type':'pointer','id':'widget-drag','parameters':{'pointerType':'mouse'},'actions':[{'type':'pointerUp','button':0}]}]})
        assert js("return !document.querySelector('.ui-drag-preview') && !document.querySelector('.widget-drop-target')"), 'Drag preview survived drop'

    # Empty the destination through host placement, then fill it with a pointer drag.
    js("for(const id of ['model-quota','notes','agent-info','session-info','usage'])localStorage.setItem(`proteus.ui.widget.${id}.position`,'composer');window.dispatchEvent(new Event('proteus-widgets-position'));window.crossZoneWidget=document.querySelector('[data-widget-id=context]');window.crossZoneRoot=crossZoneWidget.querySelector('span').shadowRoot")
    drag_widget('context','header',cancel=True)
    assert js("return document.querySelector('[data-widget-slot=composer] [data-widget-id=context]')===crossZoneWidget && localStorage.getItem('proteus.ui.widget.context.position')==='composer'"), 'Escape changed placement'
    drag_widget('context','header')
    assert js("return document.querySelector('[data-widget-slot=header] [data-widget-id=context]')===crossZoneWidget && crossZoneWidget.querySelector('span').shadowRoot===crossZoneRoot && localStorage.getItem('proteus.ui.widget.context.position')==='header'"), 'Cross-zone drag remounted or lost placement'
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('[data-widget-slot=header] [data-widget-id=context]')"), 'Cross-zone drop lost on reload')
    wait_for(lambda: js("return !!document.querySelector('[data-widget-id=session-info]')?.dataset.uiTooltipDetails"), 'Compact-only extension did not mount')
    assert js("return !document.querySelector('[data-tab-id=session-info]') && !document.querySelector('[data-extension-id=session-info]')"), 'Compact-only extension created a tab'
    js("document.querySelector('[data-widget-id=session-info]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:200,clientY:200}))")
    assert js("const p=document.querySelector('.extension-widget-menu');return p.matches(':popover-open') && p.textContent.includes('Скрыть иконку') && !p.textContent.includes('Открыть вкладку')"), 'Compact-only context menu offered tab or missed actions'
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    js("document.querySelector('[data-widget-id=context]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:200,clientY:200}))")
    assert js("return document.querySelector('.extension-widget-menu').textContent.includes('Открыть вкладку')"), 'Workspace context action missing'
    js("document.querySelector('.extension-widget-menu button:last-child').click()")
    assert js("return !document.querySelector('[data-widget-id=context]') && localStorage.getItem('proteus.ui.widget.context.position')==='hidden'"), 'Context hide did not persist'
    assert js("const n=document.querySelector('.extension-widget-notice');return n?.matches(':popover-open') && n.textContent.includes('Заполнение контекста') && n.textContent.includes('настройках')"), 'Hiding did not say where the icon went'
    js("[...document.querySelectorAll('.extension-widget-notice button')].find(b=>b.textContent==='Вернуть').click()")
    assert js("return !!document.querySelector('[data-widget-slot=header] [data-widget-id=context]') && localStorage.getItem('proteus.ui.widget.context.position')==='header' && !document.querySelector('.extension-widget-notice').matches(':popover-open')"), 'Undo did not restore the previous zone'
    js("localStorage.setItem('proteus.ui.widget.context.position','composer');window.dispatchEvent(new Event('proteus-widgets-position'))")

    # Host detail updates remain visible as text while the tooltip is open.
    js("const b=document.querySelector('[data-widget-id=plan]');window.hoverBefore=[b.dataset.uiTooltip,b.dataset.uiTooltipDetails];b.title='План: 1/2';b.dataset.uiTooltipDetails='✓ Первый шаг <b>текст</b>\\n• Текущий шаг'")
    wait_for(lambda: js("return document.querySelector('[data-widget-id=plan]')?.dataset.uiTooltipDetails.includes('Текущий шаг')"), 'Plan did not publish hover details')
    js("document.querySelector('[data-widget-id=plan]').focus()")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip')?.matches(':popover-open') && document.querySelector('.ui-tooltip').textContent.includes('Текущий шаг')"), 'Plan hover details missing')
    assert js("const t=document.querySelector('.ui-tooltip');return t.textContent.includes('План: 1/2') && t.textContent.includes('Первый шаг <b>текст</b>') && !t.querySelector('b')"), 'Plan tooltip lost steps or interpreted HTML'
    js("document.querySelector('[data-widget-id=plan]').dataset.uiTooltipDetails='• Обновлённый шаг'")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip')?.matches(':popover-open') && document.querySelector('.ui-tooltip').textContent.includes('Обновлённый шаг') && !document.querySelector('.ui-tooltip').textContent.includes('Текущий шаг')"), 'Visible tooltip did not update live')
    js("const b=document.querySelector('[data-widget-id=plan]');b.blur();b.title=hoverBefore[0];b.dataset.uiTooltipDetails=hoverBefore[1]")

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
    click('.session-item-shell .session-more')
    assert js("return document.querySelector('.sidebar-menu').getBoundingClientRect().height<360"), 'Context menu stretched to bottom'
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    js("if(document.querySelector('.tab-workspace').hidden)document.querySelector('[data-workspace-split]').click()")
    click('.workspace-add')
    assert js("const p=document.querySelector('.workspace-picker'),r=p.getBoundingClientRect(),last=p.lastElementChild.getBoundingClientRect();return r.bottom-last.bottom<16&&r.bottom<innerHeight-8"), 'Tab picker stretched to bottom'
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    print('PASS: independent per-extension placement; live roots, hide/show and persistence; write rollback; compact hover/context/tab menus',flush=True)
