"""Geometry/performance smoke on the actual client with a long synthetic transcript."""
import base64
import json
from pathlib import Path


def run(command, js, wait_for):
    js("if (!document.querySelector('.info-panel.open'))document.querySelector('.info-panel-header button').click(); window.perfFixture=document.createElement('div'); for(let i=0;i<240;i++){const p=document.createElement('article');p.className='task-card';p.textContent=('Representative transcript text with paths and code fragments. ').repeat(35);window.perfFixture.append(p)} document.querySelector('.results-panel').append(window.perfFixture)")
    for selector in ['.info-panel', '.sidebar']:
        for _ in range(2):
            result = command('/execute/async', {'script': '''const done=arguments[arguments.length-1],selector=arguments[0]; const panel=document.querySelector(selector); const widths=[],gaps=[];let last=performance.now();const start=last;panel.querySelector(selector==='.sidebar'?'.sidebar-collapse-toggle':'.info-panel-header button').click();function frame(now){gaps.push(now-last);last=now;widths.push(panel.getBoundingClientRect().width);if(now-start<320)requestAnimationFrame(frame);else done({widths:[...new Set(widths.map(Math.round))],maxFrameMs:Math.max(...gaps),frames:gaps.length})}requestAnimationFrame(frame);''', 'args': [selector]})
            print('PANEL_REFLOW', selector, json.dumps(result), flush=True)
            assert len(result['widths']) <= 2, 'Panel animates layout width across frames'
    # A large composer must never cover the last visible part of the transcript.
    js("const handle=document.querySelector('.composer-resize-handle');const y=handle.getBoundingClientRect().top;handle.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,clientY:y}));document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientY:y-150}));document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mouseup',{bubbles:true}))")
    assert js("return document.querySelector('.results-panel').getBoundingClientRect().bottom <= document.querySelector('.composer-shell').getBoundingClientRect().top"), 'Composer covers transcript after resize'
    # Persistence must run after drag, not on every movement.
    js("window.storageWrites=0;window.originalSetItem=Storage.prototype.setItem;Storage.prototype.setItem=function(...args){window.storageWrites++;return window.originalSetItem.apply(this,args)};const h=document.querySelector('.info-panel-resize-handle');window.dragX=h.getBoundingClientRect().left;h.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,clientX:window.dragX}))")
    for step in range(1, 9):
        js(f"document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mousemove',{{bubbles:true,clientX:window.dragX-{step*5}}}))")
    assert js('return window.storageWrites') == 0, 'Resize writes localStorage on each mousemove'
    js("document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mouseup',{bubbles:true}))")
    wait_for(lambda: js('return window.storageWrites > 0'), 'Resize was not persisted after release')
    js('Storage.prototype.setItem=window.originalSetItem; window.perfFixture.remove()')
    # Restore representative dimensions for screenshots.
    js("const h=document.querySelector('.composer-resize-handle');const y=h.getBoundingClientRect().top;h.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,clientY:y}));document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientY:y+150}));document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mouseup',{bubbles:true}))")
    for width in [900, 640, 390]:
        command('/window/rect', {'width': width, 'height': 1000})
        if width == 900:
            js("if (!document.querySelector('.app-layout.sidebar-collapsed'))document.querySelector('.sidebar-collapse-toggle').click()")
        assert js('return document.documentElement.scrollWidth <= window.innerWidth'), f'Horizontal overflow at {width}px'
        assert js("return document.querySelector('.sidebar').getBoundingClientRect().width > window.innerWidth - 2"), 'Collapsed mobile sidebar shrank to desktop rail width'
        js("if (!document.querySelector('.info-panel.open'))document.querySelector('.info-panel-mobile-toggle').click()")
        wait_for(lambda: js("return !!document.querySelector('.panel-backdrop')"), 'Drawer backdrop missing')
        js("document.querySelector('.panel-backdrop').click()")
        wait_for(lambda: js("return !document.querySelector('.info-panel.open')"), 'Backdrop did not dismiss drawer')
        js("document.querySelector('.info-panel-mobile-toggle').click()")
        wait_for(lambda: js("return !!document.querySelector('.info-panel.open')"), 'Drawer did not open for keyboard check')
        js("window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.info-panel.open')"), 'Escape did not dismiss drawer')
        assert js("return document.querySelector('.results-panel').getBoundingClientRect().bottom <= document.querySelector('.composer-shell').getBoundingClientRect().top"), 'Mobile composer overlaps transcript'
    command('/window/rect', {'width': 1440, 'height': 1000})
    js("if (document.querySelector('.app-layout.sidebar-collapsed'))document.querySelector('.sidebar-collapse-toggle').click();if (!document.querySelector('.info-panel.open'))document.querySelector('.info-panel-header button').click(); document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=notes]')"), 'Settings failed after layout checks')
    wait_for(lambda: js("return document.querySelector('.settings-link').classList.contains('active')"), 'Settings tab indicator stale')
    command('/execute/async', {'script': 'const done=arguments[arguments.length-1];requestAnimationFrame(()=>requestAnimationFrame(()=>done(null)))', 'args': []})
    Path('/tmp/proteus-ui-settings.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("document.querySelector('.topnav a[href=\"/\"]').click()")
    wait_for(lambda: js("return document.querySelector('[data-extension-id=model-quota] .extension-panel-content')?.shadowRoot?.textContent.includes('73% осталось')"), 'Quota failed after layout checks')
