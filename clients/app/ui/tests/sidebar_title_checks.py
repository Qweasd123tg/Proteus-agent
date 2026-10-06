"""Long saved chat names stay within the sidebar at its supported widths."""
import json


def run(command, js, wait_for):
    wait_for(lambda: js("return !!document.querySelector('.session-item-shell')"), 'Saved chat missing')
    saved = js("return [localStorage.getItem('proteus.sidebar.sessions'),localStorage.getItem('proteus.sidebarWidth')]")
    session = js("return document.querySelector('.session-item-shell').dataset.sessionDir")
    row = f"[...document.querySelectorAll('.session-item-shell')].find(x=>x.dataset.sessionDir==={json.dumps(session)})"
    title = 'ДлинноеНазваниеЧатаБезПробелов' * 5

    def menu_action(label):
        js(f"{row}.querySelector('.session-more').click()")
        js(f"[...document.querySelectorAll('.sidebar-menu button')].find(b=>b.textContent==={json.dumps(label)}).click()")

    def resize(width):
        start = js("const r=document.querySelector('.sidebar-resize-handle').getBoundingClientRect();return [Math.round(r.x+r.width/2),Math.round(r.y+r.height/2),document.querySelector('.sidebar').getBoundingClientRect().width]")
        command('/actions', {'actions': [{'type': 'pointer', 'id': 'sidebar-resize', 'parameters': {'pointerType': 'mouse'}, 'actions': [
            {'type': 'pointerMove', 'origin': 'viewport', 'x': start[0], 'y': start[1]},
            {'type': 'pointerDown', 'button': 0},
            {'type': 'pointerMove', 'duration': 150, 'origin': 'viewport', 'x': round(start[0] + width - start[2]), 'y': start[1]},
            {'type': 'pointerUp', 'button': 0},
        ]}]})
        wait_for(lambda: abs(js("return document.querySelector('.sidebar').getBoundingClientRect().width") - width) < 1, 'Sidebar resize did not settle')

    def pointer_click(selector, button):
        point = js(f"const r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return [Math.round(r.x+r.width/2),Math.round(r.y+r.height/2)]")
        command('/actions', {'actions': [{'type': 'pointer', 'id': 'sidebar-menu-mouse', 'parameters': {'pointerType': 'mouse'}, 'actions': [
            {'type': 'pointerMove', 'origin': 'viewport', 'x': point[0], 'y': point[1]},
            {'type': 'pointerDown', 'button': button},
            {'type': 'pause', 'duration': 100},
            {'type': 'pointerUp', 'button': button},
        ]}]})

    def menu_open():
        return js("return document.querySelector('.sidebar-menu')?.matches(':popover-open')")

    try:
        menu_action('Переименовать')
        js(f"const input=document.querySelector('.sidebar-rename input');input.value={json.dumps(title)};input.form.requestSubmit()")
        wait_for(lambda: js(f"return {row}.dataset.hoverTitle==={json.dumps(title)}"), 'Rename did not update chat row')
        for pinned in [False, True]:
            if pinned:
                menu_action('Закрепить')
                wait_for(lambda: js(f"return {row}.dataset.pinned==='true'"), 'Pin did not update chat row')
            for width in [210, 280, 360]:
                resize(width)
                geometry = js(f"""
                  const row={row}, list=row.closest('.session-list'), history=list.parentElement;
                  const button=row.querySelector('.session-item'), text=row.querySelector('.session-id'),
                        more=row.querySelector('.session-more'), status=row.querySelector('.session-title-line>span'), pin=row.querySelector('.session-title-line>svg');
                  const box=el=>{{const r=el.getBoundingClientRect();return {{width:r.width,left:r.left,right:r.right,client:el.clientWidth,scroll:el.scrollWidth}}}};
                  const style=getComputedStyle(text);
                  return {{width:{width},pinned:{json.dumps(pinned)},history:box(history),list:box(list),item:box(row.parentElement),row:box(row),button:box(button),text:box(text),more:box(more),status:box(status),pin:pin?box(pin):null,overflowX:getComputedStyle(history).overflowX,textOverflow:style.textOverflow,maskImage:style.maskImage,whiteSpace:style.whiteSpace,textOverflowX:style.overflowX}};
                """)
                print('Sidebar long-title geometry: ' + json.dumps(geometry, ensure_ascii=False), flush=True)
                assert geometry['history']['scroll'] <= geometry['history']['client'] + 1, 'Chat title creates horizontal history overflow'
                assert geometry['list']['scroll'] <= geometry['list']['client'] + 1, 'Chat title expands the list grid'
                assert geometry['row']['right'] <= geometry['history']['right'] + 1, 'Chat row escapes the sidebar'
                assert geometry['text']['scroll'] > geometry['text']['client'], 'Long title was not constrained'
                faded = geometry['textOverflow'] == 'clip' and geometry['maskImage'] not in ('', 'none')
                assert (geometry['textOverflow'] == 'ellipsis' or faded) and geometry['whiteSpace'] == 'nowrap' and geometry['textOverflowX'] == 'hidden', 'Title is not truncated to one line with an ellipsis or fade'
                assert geometry['text']['right'] <= geometry['more']['left'] + 1, 'Title overlaps the actions button'
                assert geometry['status']['width'] > 0 and geometry['status']['right'] <= geometry['text']['left'] + 1, 'Title hides the status dot'
                if pinned:
                    assert geometry['pin'] and geometry['pin']['width'] > 0, 'Title hides the pin icon'
                    assert geometry['text']['right'] <= geometry['pin']['left'] + 1 and geometry['pin']['right'] <= geometry['more']['left'] + 1, 'Pin overlaps the title or actions button'
                js(f"{row}.querySelector('.session-item').focus()")
                wait_for(lambda: js(f"return document.querySelector('.sidebar-hover')?.matches(':popover-open') && document.querySelector('.sidebar-hover strong')?.textContent==={json.dumps(title)}"), 'Full chat title missing from hover')
                js("document.activeElement.blur()")
        command('/refresh', {})
        wait_for(lambda: js(f"return {row}?.dataset.hoverTitle==={json.dumps(title)} && {row}.dataset.pinned==='true'"), 'Renamed/pinned chat did not survive reload')
        # Exercise actual secondary-button press/release; synthetic contextmenu
        # alone cannot catch a popup disappearing on mouse release.
        pointer_click('.sidebar-project', 2)
        wait_for(menu_open, 'Project context menu closed on right-button release')
        assert js("return document.querySelector('.sidebar-menu').textContent.includes('Новый чат')"), 'Project right click opened the wrong menu'
        pointer_click('.composer textarea', 0)
        wait_for(lambda: not menu_open(), 'Outside click did not close project menu')
        pointer_click('.sidebar-project', 2)
        wait_for(menu_open, 'Project context menu did not reopen')
        command('/actions', {'actions': [{'type': 'key', 'id': 'sidebar-menu-keyboard', 'actions': [{'type': 'keyDown', 'value': '\ue00c'}, {'type': 'keyUp', 'value': '\ue00c'}]}]})
        wait_for(lambda: not menu_open(), 'Escape did not close project menu')
        js("window.sidebarContextPrevented=false;document.addEventListener('contextmenu',event=>{if(event.target.closest('.session-item-shell'))window.sidebarContextPrevented=event.defaultPrevented},{once:true})")
        pointer_click('.session-item', 2)
        assert js("return window.sidebarContextPrevented") and not menu_open(), 'Chat right click showed a menu or allowed native Inspect'
        js(f"{row}.querySelector('.session-more').click()")
        wait_for(menu_open, 'Chat actions button did not open its menu')
        assert js("return document.querySelector('.sidebar-menu').textContent.includes('Переименовать') && document.querySelector('.sidebar-menu').textContent.includes('Открепить')"), 'Chat actions lost rename or pin controls'
        js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
        print('PASS: saved chat titles truncate at 210/280/360 px; pin/status/actions stay visible; full hover name and rename survive reload', flush=True)
        print('PASS: project right click survives press/release; outside click and Escape close; chat right click suppressed; chat actions remain under …', flush=True)
    finally:
        js(f"for(const [key,value] of {json.dumps(list(zip(['proteus.sidebar.sessions','proteus.sidebarWidth'], saved)))}){{if(value===null)localStorage.removeItem(key);else localStorage.setItem(key,value)}}")
        command('/refresh', {})
        wait_for(lambda: js("return !!document.querySelector('.session-item-shell') && !!document.querySelector('[data-widget-id=plan]')"), 'Sidebar fixture did not restore')
