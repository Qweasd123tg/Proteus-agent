"""Shared tabs: browser-owned roots, split geometry, persistence and controls."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    def click(selector):
        js('document.querySelector('+repr(selector)+').click()')
    def point(selector):
        return js('const r=document.querySelector('+repr(selector)+').getBoundingClientRect();return [Math.round(r.x+r.width/2),Math.round(r.y+r.height/2)]')
    def pointer(actions):
        command('/actions', {'actions':[{'type':'pointer','id':'workspace-mouse','parameters':{'pointerType':'mouse'},'actions':actions}]})
    def move(point):
        return {'type':'pointerMove','duration':120,'origin':'viewport','x':point[0],'y':point[1]}
    def pick(id, group=0):
        click(f'.workspace-group[data-group="{group}"] .workspace-add')
        click(f'.workspace-picker [data-open-tab="{id}"]')
    def tab(id):
        return f'[data-tab-id="{id}"] .workspace-tab-name'
    def restored_split_geometry():
        return js("const board=document.querySelector('.tab-workspace'),first=board.querySelector('[data-group=\"0\"]'),expected=JSON.parse(savedLayout).ratio;return Math.abs(parseFloat(getComputedStyle(board).getPropertyValue('--workspace-ratio'))/100-expected)<0.0001 && Math.abs(first.getBoundingClientRect().width-(board.getBoundingClientRect().width*expected-3))<1")
    def save_bindings(body):
        # Firefox WebDriver imports use a separate module map. Execute this in
        # the page realm, as the settings UI does, to update its live runtime.
        module = "import * as m from '/ui/shortcuts/runtime.js';" + body
        result = command('/execute/async', {
            'script': """const done=arguments[arguments.length-1];
                const script=document.createElement('script');script.type='module';
                window.addEventListener('workspace-bindings-saved',event=>{script.remove();done(event.detail)},{once:true});
                script.textContent=arguments[0]+";window.dispatchEvent(new CustomEvent('workspace-bindings-saved',{detail:null}))";
                script.onerror=()=>{script.remove();done('Shortcut module failed to load')};
                document.head.append(script);""",
            'args': [module],
        })
        assert result is None, result
    wait_for(lambda: js("return !!document.querySelector('[data-tab-id=\"client:chat\"]') && !!document.querySelector('.composer-model-menu')"),'Client tabs missing')
    js("window.keptChat=document.querySelector('.session-workspace');window.keptComposer=document.querySelector('.composer textarea');keptComposer.value='Черновик между областями';keptComposer.dispatchEvent(new Event('input',{bubbles:true}));window.keptModel=document.querySelector('.composer-model-menu')")
    assert js("return !document.querySelector('[data-tab-id=\"client:settings\"]')"),'Settings became a workspace tab'
    click('.workspace-group[data-group="0"] .workspace-add')
    wait_for(lambda: js("return !!document.querySelector('.workspace-picker [data-open-tab=files]')"),'Files tab choice missing')
    assert js("return !document.querySelector('.workspace-picker [data-open-tab=\"client:settings\"]')"),'Settings leaked into the tab picker'
    click('.workspace-picker [data-open-tab=files]')
    wait_for(lambda: js("return document.querySelector('[data-tab-id=files]')?.classList.contains('active') && !!document.querySelector('[data-extension-id=files] .extension-panel-content')"),'Files tab did not open')
    # Hover paints the complete inactive tab; the close button never disappears.
    pointer([move([2, 2])])
    js("document.activeElement.blur();window.hoverTab=document.querySelector('[data-tab-id=\"client:chat\"]');window.clearBackground=color=>color==='transparent'||color==='rgba(0, 0, 0, 0)'")
    wait_for(lambda: js("return !hoverTab.matches(':hover,:focus-within') && clearBackground(getComputedStyle(hoverTab).backgroundColor)"),'Inactive tab did not settle before hover')
    js("window.idleTabBackground=getComputedStyle(hoverTab).backgroundColor;window.idleTabWidth=hoverTab.getBoundingClientRect().width")
    assert js("return getComputedStyle(hoverTab.querySelector('.workspace-tab-close')).opacity==='1'"),'Inactive close button is hidden without hover'
    pointer([move(point(tab('client:chat')))])
    wait_for(lambda: js("return hoverTab.matches(':hover') && getComputedStyle(hoverTab).backgroundColor!==idleTabBackground"),'Pointer hover did not highlight the whole inactive tab')
    assert js("return [...hoverTab.children].every(button=>clearBackground(getComputedStyle(button).backgroundColor))"),'Tab hover paints a child button instead of the wrapper'
    pointer([move(point('[data-tab-id="client:chat"] .workspace-tab-close'))])
    assert js("return hoverTab.matches(':hover') && clearBackground(getComputedStyle(hoverTab.querySelector('.workspace-tab-close')).backgroundColor) && getComputedStyle(hoverTab.querySelector('.workspace-tab-close')).opacity==='1'"),'Close-button hover breaks the shared tab highlight'
    pointer([move([2, 2])])
    wait_for(lambda: js("return getComputedStyle(hoverTab).backgroundColor===idleTabBackground"),'Tab hover did not clear')
    assert js("return getComputedStyle(hoverTab.querySelector('.workspace-tab-close')).opacity==='1' && hoverTab.getBoundingClientRect().width===idleTabWidth"),'Leaving hover hid the close button or shifted the tab'
    js("window.keptTool=document.querySelector('[data-extension-id=files]');window.keptToolRoot=keptTool.querySelector('.extension-panel-content').shadowRoot")
    click('.workspace-group[data-group="0"] .workspace-transfer')
    assert js("return document.querySelectorAll('.workspace-group:not([hidden])').length===2 && !keptChat.hidden && !keptTool.hidden && document.querySelector('[data-tab-id=files]').closest('[data-group]').dataset.group==='1'"),'Transfer did not split chat and tool'
    assert js("return document.querySelector('.composer textarea')===keptComposer && keptComposer.value==='Черновик между областями'"),'Transfer lost draft or composer root'
    # An iframe must keep its document when its tab moves: DOM reparent reloads it.
    js("window.keepFrame=document.createElement('iframe');keepFrame.srcdoc='<input value=original>';keptTool.append(keepFrame)")
    wait_for(lambda: js("return !!keepFrame.contentDocument?.querySelector('input')"),'Iframe probe missing')
    js("window.keepDocument=keepFrame.contentDocument;keepDocument.querySelector('input').value='state'")
    click('.workspace-group[data-group="1"] .workspace-transfer')
    assert js("return keepFrame.contentDocument===keepDocument && keepDocument.querySelector('input').value==='state' && keptTool===document.querySelector('[data-extension-id=files]') && keptTool.querySelector('.extension-panel-content').shadowRoot===keptToolRoot"),'Moving tool reloaded its document'
    click('.workspace-group[data-group="0"] .workspace-transfer')
    js("keepFrame.remove()")
    js("window.beforeRatio=JSON.parse(localStorage.getItem('proteus.workspace.layout')).ratio;document.querySelector('.workspace-resize').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}))")
    assert js("return JSON.parse(localStorage.getItem('proteus.workspace.layout')).ratio<beforeRatio"),'Divider did not persist'
    # Drag the chat into the other tab strip and cancel a second drag.
    start=point(tab('client:chat'));end=point('.workspace-group[data-group="1"] .workspace-tabs')
    pointer([move(start),{'type':'pointerDown','button':0},move(end),{'type':'pointerUp','button':0}])
    assert js("return document.querySelector('[data-tab-id=\"client:chat\"]').closest('[data-group]').dataset.group==='1' && keptComposer.value==='Черновик между областями'"),'Pointer transfer failed'
    js("window.savedLayout=localStorage.getItem('proteus.workspace.layout')")
    start=point(tab('client:chat'));end=point('.workspace-group[data-group="0"] .workspace-tabs')
    pointer([move(start),{'type':'pointerDown','button':0},move(end)])
    js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    pointer([{'type':'pointerUp','button':0}])
    assert js("return localStorage.getItem('proteus.workspace.layout')===savedLayout && !document.querySelector('.ui-drag-preview')"),'Cancel saved a partial drag'
    # Settings is an independent retained screen; it cannot take a workspace slot.
    click(tab('files'))
    js("window.savedLayout=localStorage.getItem('proteus.workspace.layout')")
    click('.settings-link')
    wait_for(lambda: js("const settings=document.querySelector('[data-client-view=settings]'),board=document.querySelector('[data-client-workspace]');return settings && !settings.hidden && !settings.inert && !!settings.querySelector('.settings-page input') && board.hidden && board.inert"),'Settings did not replace the workspace screen')
    assert js("return !document.querySelector('[data-client-view=settings]').closest('[data-client-workspace]') && !document.querySelector('[data-tab-id=\"client:settings\"]') && localStorage.getItem('proteus.workspace.layout')===savedLayout"),'Opening settings changed the workspace layout'
    js("window.keptSettings=document.querySelector('.settings-page');window.keptSettingControl=keptSettings.querySelector('input')")
    click('.settings-back')
    wait_for(lambda: js("const board=document.querySelector('[data-client-workspace]'),settings=document.querySelector('[data-client-view=settings]');return !board.hidden && !board.inert && settings.hidden && settings.inert"),'Returning to chat did not restore workspace')
    wait_for(restored_split_geometry,'Returning from settings changed the visible split ratio')
    assert js("return localStorage.getItem('proteus.workspace.layout')===savedLayout && document.querySelector('[data-tab-id=files]').classList.contains('active') && keptChat.hidden && document.querySelector('.session-workspace')===keptChat && document.querySelector('.composer textarea')===keptComposer && keptComposer.value==='Черновик между областями' && document.querySelector('[data-extension-id=files]')===keptTool && keptTool.querySelector('.extension-panel-content').shadowRoot===keptToolRoot"),'Settings round trip reset layout, selected tool or draft'
    click('.settings-link')
    wait_for(lambda: js("return !document.querySelector('[data-client-view=settings]').hidden"),'Settings did not reopen')
    assert js("return document.querySelector('.settings-page')===keptSettings && keptSettings.querySelector('input')===keptSettingControl"),'Settings screen was remounted'
    click('.settings-back')
    wait_for(lambda: js("return !document.querySelector('[data-client-workspace]').hidden"),'Workspace did not reopen')
    wait_for(restored_split_geometry,'Reopening settings changed the visible split ratio')
    assert js("return localStorage.getItem('proteus.workspace.layout')===savedLayout && document.querySelector('[data-tab-id=files]').classList.contains('active')"),'Reopening settings lost the selected tool'
    click(tab('client:chat'))
    click('.workspace-group[data-group="0"] .workspace-add')
    assert js("return !document.querySelector('.workspace-picker [data-open-tab=\"client:settings\"]')"),'Visited settings leaked into the tab picker'
    js("document.querySelector('.workspace-picker').hidePopover()")
    # Persistent slot placement uses the same roots and opens a bounded menu up top.
    grip='[data-control-id=composer-model] .module-drag-handle'
    click(grip)
    js("[...document.querySelectorAll('.module-placement-menu button')].find(b=>b.textContent==='В верхней панели').click()")
    assert js("return document.querySelector('[data-module-zone=header] .composer-model-menu')===keptModel"),'Moving model replaced module'
    click('.composer-model-menu summary')
    wait_for(lambda: js("return document.querySelector('.composer-model-menu .composer-menu-panel').matches(':popover-open')"),'Moved selector did not open')
    assert js("const r=document.querySelector('.composer-model-menu .composer-menu-panel').getBoundingClientRect();return r.top>=0 && r.bottom<=innerHeight && r.right<=innerWidth"),'Moved menu is outside viewport'
    js("document.querySelector('.composer-model-menu').open=false")
    # Actual drag between zones, then persist header placement again.
    start=point(grip);end=point('[data-module-zone=composer-start]')
    pointer([move(start),{'type':'pointerDown','button':0},move(end),{'type':'pointerUp','button':0}])
    assert js("return document.querySelector('[data-module-zone=composer-start] .composer-model-menu')===keptModel"),'Control pointer transfer failed'
    js("document.querySelector('[data-control-id=composer-model] .module-drag-handle').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',altKey:true,bubbles:true}))")
    assert js("const zone=document.querySelector('[data-module-zone=composer-start]');return zone.firstElementChild.dataset.controlId==='composer-model' && JSON.parse(localStorage.getItem('proteus.client.controls.layout')).filter(x=>x.zone==='composer-start')[0].id==='composer-model'"),'Keyboard control reorder did not apply or persist'
    click(grip);js("[...document.querySelectorAll('.module-placement-menu button')].find(b=>b.textContent==='В верхней панели').click()")
    # Tooltips reflect a live rebinding, and omit disabled bindings.
    pointer([move([2, 2])])
    js("document.querySelector('[data-workspace-split]').focus()")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip')?.matches(':popover-open') && document.querySelector('.ui-tooltip').textContent.includes('Ctrl + Shift + B')"),'Shortcut absent from tooltip')
    js("document.querySelector('.composer textarea').dispatchEvent(new PointerEvent('pointerout',{bubbles:true,relatedTarget:document.body}))")
    assert js("return document.querySelector('.ui-tooltip').matches(':popover-open')"),'Unrelated pointer exit hid the focused tooltip'
    save_bindings("window.savedBindings=m.snapshot().bindings;m.save({...savedBindings,workspace:'Alt+KeyB'})")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip').textContent.includes('Alt + B') && !document.querySelector('.ui-tooltip').textContent.includes('Ctrl + Shift + B')"),'Tooltip binding is stale')
    save_bindings("m.save({...savedBindings,workspace:null})")
    assert js("return !document.querySelector('.ui-tooltip').textContent.includes('Alt + B')"),'Disabled shortcut still advertised'
    save_bindings("m.save(savedBindings)")
    # A module may expose its accessible label without a native title.
    js("window.shortcutProbe=document.createElement('button');shortcutProbe.dataset.shortcut='workspace';shortcutProbe.setAttribute('aria-label','Команда модуля');document.querySelector('.topnav').append(shortcutProbe);shortcutProbe.focus()")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip').matches(':popover-open') && document.querySelector('.ui-tooltip').textContent.includes('Команда модуля') && document.querySelector('.ui-tooltip').textContent.includes('Ctrl + Shift + B')"),'Accessible module shortcut label missing')
    js("shortcutProbe.dataset.shortcut='sidebar';shortcutProbe.setAttribute('aria-label','Другая команда')")
    wait_for(lambda: js("return document.querySelector('.ui-tooltip').textContent.includes('Другая команда') && document.querySelector('.ui-tooltip').textContent.includes('Ctrl + B') && !document.querySelector('.ui-tooltip').textContent.includes('Shift')"),'Changed shortcut target retained stale hint')
    js("shortcutProbe.remove()")
    wait_for(lambda: js("return !document.querySelector('.ui-tooltip').matches(':popover-open')"),'Removed module retained tooltip')
    Path('/tmp/proteus-workspace-split.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    command('/refresh',{})
    wait_for(lambda: js("return !!document.querySelector('[data-module-zone=header] .composer-model-menu')"),'Control layout not restored')
    assert js("return document.querySelectorAll('.workspace-group:not([hidden])').length===2 && document.querySelector('[data-tab-id=\"client:chat\"]').closest('[data-group]').dataset.group==='1'"),'Workspace layout not restored'
    click('[data-workspace-split]')
    assert js("return document.querySelectorAll('.workspace-group:not([hidden])').length===1 && !!document.querySelector('[data-tab-id=\"client:chat\"]')"),'Merge lost a client tab'
    pick('client:chat')
    # Closing a tab must leave the saved view available from the picker.
    js("window.lastChat=document.querySelector('.session-workspace')")
    click('[data-tab-id="client:chat"] .workspace-tab-close');pick('client:chat')
    assert js("return document.querySelector('.session-workspace')===lastChat && !lastChat.hidden"),'Close/reopen destroyed client root'
    # Direct /settings startup and browser history preserve the selected tool.
    pick('files')
    history_layout = js("return localStorage.getItem('proteus.workspace.layout')")
    def settings_screen():
        return js("const settings=document.querySelector('[data-client-view=settings]'),board=document.querySelector('[data-client-workspace]');return location.pathname==='/settings' && settings && !settings.hidden && !settings.inert && !!settings.querySelector('.settings-back') && board?.hidden && board.inert && !document.querySelector('[data-tab-id=\"client:settings\"]')")
    def files_workspace():
        return js("const settings=document.querySelector('[data-client-view=settings]'),board=document.querySelector('[data-client-workspace]');return location.pathname==='/' && board && !board.hidden && !board.inert && settings?.hidden && settings.inert && document.querySelector('[data-tab-id=files]')?.classList.contains('active')")
    click('.settings-link')
    wait_for(settings_screen,'Settings route did not open before reload')
    command('/refresh',{})
    wait_for(settings_screen,'Direct /settings reload did not keep settings separate from workspace')
    js("window.historyLayout="+repr(history_layout)+";window.historySettings=document.querySelector('.settings-page');window.historyChat=document.querySelector('.session-workspace')")
    assert js("return localStorage.getItem('proteus.workspace.layout')===historyLayout"),'Direct settings startup changed the saved workspace'
    click('.settings-back')
    wait_for(files_workspace,'Back from directly loaded settings did not restore the selected tool')
    assert js("return localStorage.getItem('proteus.workspace.layout')===historyLayout"),'Direct settings return changed the previous layout'
    js("window.historyTool=document.querySelector('[data-extension-id=files]')")
    command('/back',{})
    wait_for(settings_screen,'Browser Back did not reopen the separate settings screen')
    assert js("return localStorage.getItem('proteus.workspace.layout')===historyLayout && document.querySelector('.settings-page')===historySettings"),'Browser Back reset layout or settings DOM'
    command('/forward',{})
    wait_for(files_workspace,'Browser Forward did not restore the selected tool workspace')
    assert js("return localStorage.getItem('proteus.workspace.layout')===historyLayout && document.querySelector('.session-workspace')===historyChat && document.querySelector('[data-extension-id=files]')===historyTool && !document.querySelector('[data-tab-id=\"client:settings\"]')"),'Browser Forward reset layout or retained views'
    print('PASS: chat/tool split, whole-tab hover and stable close button, independent retained settings with direct startup and browser history, pointer transfer/cancel, iframe document and draft identity, divider, merge, reload, control drag/menu/placement, live shortcut hints',flush=True)
