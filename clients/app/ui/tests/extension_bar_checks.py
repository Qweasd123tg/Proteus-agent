"""Trusted pointer drops, persistent order and compact themed tooltips."""
import json


def run(command, js, wait_for):
    def move(x, y):
        return {'type': 'pointerMove', 'duration': 100, 'origin': 'viewport', 'x': round(x), 'y': round(y)}

    def pointer(actions):
        command('/actions', {'actions': [{'type': 'pointer', 'id': 'extension-bars', 'parameters': {'pointerType': 'mouse'}, 'actions': actions}]})

    def point(selector, edge=False):
        return js(f"const r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return [r.left+{'2' if edge else 'r.width/2'},r.top+r.height/2]")

    def order():
        return js("return [...document.querySelector('[data-widget-slot=composer] .extension-widgets').children].map(b=>b.dataset.widgetId)")

    wait_for(lambda: js("return document.querySelectorAll('[data-widget-slot=composer] .extension-widget').length>=3"), 'Widget strip missing')
    assert js("return document.querySelectorAll('[data-workspace-split]').length===1 && !document.querySelector('.workspace-tabbar [aria-label=\"Свернуть боковую панель\"]')"), 'Workspace toggle duplicated'
    js("window.keptWidgets=[...document.querySelectorAll('.extension-widget')];window.keptPanels=[...document.querySelectorAll('[data-extension-id]')];window.tabActive=document.querySelector('.workspace-tab.active')?.dataset.tabId;window.savedOrder=localStorage.getItem('proteus.ui.extensions');document.querySelector('[data-widget-slot=composer] .extension-widgets').scrollLeft=0")
    before = order()
    dragged = f'[data-widget-id="{before[2]}"]'
    target = f'[data-widget-id="{before[0]}"]'
    pointer([move(*point(dragged)), {'type': 'pointerDown', 'button': 0}, move(*point(target, True))])
    assert js("return !!document.querySelector('.ui-drag-preview') && !!document.querySelector('.ui-drag-preview span')?.shadowRoot && localStorage.getItem('proteus.ui.extensions')===savedOrder"), 'Widget preview missing or saved before drop'
    pointer([{'type': 'pointerUp', 'button': 0}])
    expected = [before[2], before[0], before[1], *before[3:]]
    assert order() == expected, 'Widget drop did not insert'
    assert js("return keptWidgets.every(b=>b.isConnected) && keptPanels.every(p=>p.isConnected) && document.querySelector('.workspace-tab.active')?.dataset.tabId===tabActive"), 'Widget drag remounted roots or triggered click'
    saved = js("return JSON.parse(localStorage.getItem('proteus.ui.extensions')).panels.filter(p=>p.enabled).map(p=>p.id)")
    assert saved == expected, 'Widget order not persisted'

    # Cancel a preview and check that it restores both DOM and saved order.
    js("document.querySelector('[data-widget-slot=composer] .extension-widgets').scrollLeft=0;window.savedOrder=localStorage.getItem('proteus.ui.extensions')")
    pointer([move(*point(dragged)), {'type': 'pointerDown', 'button': 0}, move(*point(f'[data-widget-id="{expected[2]}"]'))])
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    pointer([{'type': 'pointerUp', 'button': 0}])
    assert order() == expected and js("return !document.querySelector('.ui-drag-preview') && localStorage.getItem('proteus.ui.extensions')===savedOrder"), 'Escape did not cancel widget drag'

    js("document.querySelector('[data-widget-slot=composer] .extension-widgets').scrollLeft=0;document.addEventListener('pointerdown',e=>window.dragPointer=e.pointerId,{once:true,capture:true})")
    pointer([move(*point(dragged)), {'type': 'pointerDown', 'button': 0}, move(*point(f'[data-widget-id="{expected[2]}"]'))])
    js("document.dispatchEvent(new PointerEvent('pointercancel',{pointerId:dragPointer+100,bubbles:true}))")
    assert js("return !!document.querySelector('.ui-drag-preview')"), 'Another pointer canceled this drag'
    js("document.dispatchEvent(new PointerEvent('pointercancel',{pointerId:window.dragPointer,bubbles:true}));window.activationAfterCancel=0;const b=document.querySelector('[data-widget-slot=composer] .extension-widget[data-widget-id]');b.addEventListener('click',()=>window.activationAfterCancel++,{once:true});b.click()")
    canceled = js("return {ghost:!!document.querySelector('.ui-drag-preview'),activated:window.activationAfterCancel,pointer:window.dragPointer}")
    assert not canceled['ghost'] and canceled['activated'] == 1, 'Pointer cancellation swallowed keyboard activation: ' + str(canceled)
    pointer([{'type': 'pointerUp', 'button': 0}])
    js("document.querySelector('[data-tab-id=\"client:chat\"] .workspace-tab-name').click()")
    assert order() == expected, 'Pointer cancellation changed order'

    # Tabs have their own group order; widget placement remains independent.
    widget_expected = order()
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('[data-widget-slot=composer] .extension-widgets')"), 'Widget strip did not reload')
    js("document.querySelector('[data-tab-id=\"client:chat\"] .workspace-tab-name').click()")
    assert order() == widget_expected, 'Reload lost widget order'

    # Hover/focus use one styled, bounded tooltip and preserve existing descriptions.
    control = '.topbar [data-workspace-split]'
    pointer([move(*point(control))])
    wait_for(lambda: js("return document.querySelector('.ui-tooltip')?.matches(':popover-open')"), 'Themed hover tooltip missing')
    assert js("const t=document.querySelector('.ui-tooltip'),r=t.getBoundingClientRect(),c=document.querySelector('.topbar [data-workspace-split]');return !c.hasAttribute('title') && t.textContent.includes('Разделить область') && t.textContent.includes('Ctrl + Shift + B') && r.height<70 && r.width<=280 && r.left>=8 && r.top>=8 && r.right<=innerWidth-8 && r.bottom<=innerHeight-8"), 'Tooltip retained native title or overflowed viewport'
    js("window.tooltipEscape=0;window.watchTooltipEscape=e=>{if(e.key==='Escape')window.tooltipEscape++};window.addEventListener('keydown',watchTooltipEscape);document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    assert js("return !document.querySelector('.ui-tooltip').matches(':popover-open') && window.tooltipEscape===0"), 'Tooltip Escape reached global cancellation'
    js("window.removeEventListener('keydown',watchTooltipEscape)")
    pointer([move(10, 10)])
    js("document.querySelector('.topbar [data-workspace-split]').setAttribute('aria-describedby','existing-description');document.querySelector('.topbar [data-workspace-split]').focus()")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip')?.matches(':popover-open')"), 'Focus tooltip missing')
    js("document.querySelector('.topbar [data-workspace-split]').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}))")
    assert js("return !document.querySelector('.ui-tooltip').matches(':popover-open') && document.querySelector('.topbar [data-workspace-split]').getAttribute('aria-describedby')==='existing-description'"), 'Pointer start left tooltip or removed description'
    print('PASS: one workspace toggle; pointer widget/tab insertion, cancel, live roots and reload; truncated names; themed hover/focus tooltip', flush=True)
