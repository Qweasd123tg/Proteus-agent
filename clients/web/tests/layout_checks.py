"""Geometry/performance smoke on the actual client with a long synthetic transcript."""
import base64
import json
from pathlib import Path


def run(command, js, wait_for):
    # Separate controls share one popup at a time; outside clicks dismiss either.
    for menu in ['access', 'model', 'access']:
        js(f"document.querySelector('.composer-{menu}-menu summary').click()")
        wait_for(lambda: js(f"return document.querySelectorAll('.composer-menu[open]').length===1 && document.querySelector('.composer-{menu}-menu').open"), 'Composer menus overlap')
    js("document.querySelector('.composer textarea').click()")
    assert js("return !document.querySelector('.composer-menu[open]')"), 'Input click did not dismiss composer menu'
    js("if (!document.querySelector('.info-panel.open'))[...document.querySelectorAll('[data-panel-toggle=info]')].find(b=>!b.closest('[inert]')).click(); window.perfFixture=document.createElement('div'); for(let i=0;i<240;i++){const p=document.createElement('article');p.className='task-card';p.textContent=('Representative transcript text with paths and code fragments. ').repeat(35);window.perfFixture.append(p)} document.querySelector('.results-panel').append(window.perfFixture)")
    for selector in ['.info-panel', '.sidebar']:
        for _ in range(2):
            result = command('/execute/async', {'script': r"""
                const done=arguments[arguments.length-1], panel=document.querySelector(arguments[0]);
                const surface=panel.querySelector('.info-panel-surface, .sidebar-surface');
                const button=[...panel.querySelectorAll('[data-panel-toggle]')].find(b=>!b.closest('[inert]'));
                const widths=[], surfaces=[], positions=[], gaps=[];
                let last=performance.now(); const start=last;
                button.focus(); button.click();
                function frame(now) {
                    gaps.push(now-last); last=now;
                    widths.push(Math.round(panel.getBoundingClientRect().width));
                    surfaces.push(Math.round(surface.getBoundingClientRect().width));
                    positions.push(Math.round(surface.getBoundingClientRect().x));
                    if(now-start<360) requestAnimationFrame(frame);
                    else done({widths:[...new Set(widths)],surfaces:[...new Set(surfaces)],positions:[...new Set(positions)],maxFrameMs:Math.max(...gaps)});
                }
                requestAnimationFrame(now => {
                    // Software rendering may skip an entire short transition
                    // between samples. Inspect a real intermediate animation
                    // frame explicitly, then let normal playback finish.
                    getComputedStyle(surface).transform;
                    const slide=surface.getAnimations().find(a=>a.transitionProperty==='transform');
                    if(slide) {
                        positions.push(Math.round(surface.getBoundingClientRect().x));
                        slide.pause(); slide.currentTime=slide.effect.getTiming().duration/2;
                        positions.push(Math.round(surface.getBoundingClientRect().x));
                        slide.play();
                    }
                    frame(now);
                });
            """, 'args': [selector]})
            print('PANEL_REFLOW', selector, json.dumps(result), flush=True)
            assert len(result['widths']) <= 2, 'Panel animates layout width across frames'
            assert len(result['surfaces']) == 1, 'Panel content changes width during slide'
            if not js("return matchMedia('(prefers-reduced-motion: reduce)').matches"):
                assert len(result['positions']) > 2, 'Panel surface did not slide'
            else:
                assert len(result['positions']) <= 2, 'Panel ignores reduced-motion preference'
            assert js("return document.activeElement.matches('[data-panel-toggle]') && !document.activeElement.closest('[inert]')"), 'Focus was lost inside the hidden panel'
    # Reverse the transition before it finishes; no stale transforms may remain.
    command('/execute/async', {'script': r"""
        const done=arguments[arguments.length-1];
        const toggle=()=>[...document.querySelectorAll('.info-panel [data-panel-toggle]')].find(b=>!b.closest('[inert]')).click();
        toggle(); setTimeout(()=>{toggle();setTimeout(done,360)},60);
    """, 'args': []})
    assert js("return document.querySelector('.info-panel').classList.contains('open') && !document.getAnimations().some(a=>a.id==='panel-layout')"), 'Rapid reversal left stale layout motion'

    def dock_clear():
        return js("const r=document.querySelector('.results-panel'), dock=document.querySelector('.composer'), shell=document.querySelector('.composer-shell'), last=r.lastElementChild;return Math.abs(r.getBoundingClientRect().bottom-dock.getBoundingClientRect().bottom)<1 && Math.abs(dock.getBoundingClientRect().top-shell.getBoundingClientRect().top)<1 && last.getBoundingClientRect().bottom <= shell.getBoundingClientRect().top")
    # The input grows for multiline drafts, caps long pastes and shrinks when cleared.
    def draft(value):
        js("const area=document.querySelector('.composer textarea');area.value=" + json.dumps(value) + ";area.dispatchEvent(new Event('input',{bubbles:true}))")
    draft('\n'.join(['Строка задачи'] * 30))
    wait_for(lambda: js("return document.querySelector('.composer textarea').clientHeight > 150"), 'Multiline composer did not grow')
    assert js("const area=document.querySelector('.composer textarea');return area.clientHeight <= 240 && area.scrollHeight > area.clientHeight"), 'Long paste was not capped and scrollable'
    wait_for(lambda: js("const dock=document.querySelector('.composer');return Math.abs(parseFloat(getComputedStyle(dock.closest('.session-workspace')).getPropertyValue('--composer-inset'))-dock.getBoundingClientRect().height)<1"), 'Dock inset did not follow input growth')
    js("const r=document.querySelector('.results-panel');r.scrollTop=r.scrollHeight")
    assert dock_clear(), 'Dock covers the final message or adds a strip above the input'
    assert js("const r=document.querySelector('.composer-shell').getBoundingClientRect();return !!document.elementFromPoint(r.left+4,r.top-2)?.closest('.results-panel')"), 'Backdrop begins above the actual input'
    Path('/tmp/proteus-ui-dock-scroll.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true}));r.scrollTop=1000")
    draft('')
    wait_for(lambda: js("return document.querySelector('.composer textarea').clientHeight < 80"), 'Cleared composer did not shrink')
    assert js("return Math.abs(document.querySelector('.results-panel').scrollTop-1000)<2"), 'Input resize pulled the reader away from older messages'
    Path('/tmp/proteus-ui-dock-overlap.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    # Persistence must run after drag, not on every movement.
    js("window.storageWrites=0;window.originalSetItem=Storage.prototype.setItem;Storage.prototype.setItem=function(...args){window.storageWrites++;return window.originalSetItem.apply(this,args)};const h=document.querySelector('.info-panel-resize-handle');window.dragX=h.getBoundingClientRect().left;h.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,clientX:window.dragX}))")
    for step in range(1, 9):
        js(f"document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mousemove',{{bubbles:true,clientX:window.dragX-{step*5}}}))")
    assert js('return window.storageWrites') == 0, 'Resize writes localStorage on each mousemove'
    js("document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mouseup',{bubbles:true}))")
    wait_for(lambda: js('return window.storageWrites > 0'), 'Resize was not persisted after release')
    js('Storage.prototype.setItem=window.originalSetItem; window.perfFixture.remove()')
    for width in [900, 640, 390]:
        command('/window/rect', {'width': width, 'height': 1000})
        if width == 900:
            js("if (!document.querySelector('.app-layout.sidebar-collapsed'))[...document.querySelectorAll('[data-panel-toggle=sidebar]')].find(b=>!b.closest('[inert]')).click()")
        assert js('return document.documentElement.scrollWidth <= window.innerWidth'), f'Horizontal overflow at {width}px'
        assert js("return document.querySelector('.sidebar').getBoundingClientRect().width > window.innerWidth - 2"), 'Collapsed mobile sidebar shrank to desktop rail width'
        js("if (!document.querySelector('.info-panel.open'))document.querySelector('.info-panel-mobile-toggle').click()")
        wait_for(lambda: js("return !!document.querySelector('.panel-backdrop.open')"), 'Drawer backdrop missing')
        js("document.querySelector('.panel-backdrop.open').click()")
        wait_for(lambda: js("return !document.querySelector('.info-panel.open')"), 'Backdrop did not dismiss drawer')
        js("document.querySelector('.info-panel-mobile-toggle').click()")
        wait_for(lambda: js("return !!document.querySelector('.info-panel.open')"), 'Drawer did not open for keyboard check')
        js("window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.info-panel.open')"), 'Escape did not dismiss drawer')
        js("const r=document.querySelector('.results-panel');r.scrollTop=r.scrollHeight")
        assert dock_clear(), 'Mobile dock hides the final message'
        assert js("return document.querySelector('.info-panel-surface').inert"), 'Closing drawer still accepts keyboard focus'
        draft('длинный_путь_без_пробелов/' * 30)
        wait_for(lambda: js("return document.querySelector('.composer textarea').clientHeight > 100"), 'Mobile composer did not wrap long text')
        assert js("const area=document.querySelector('.composer textarea');return area.scrollWidth <= area.clientWidth && document.documentElement.scrollWidth <= innerWidth"), 'Long draft overflows horizontally'
        # Exercise the toolbar with a long model label and visible reasoning level.
        js("const label=document.querySelector('.composer-model-menu .composer-menu-model');window.originalModelLabel=label.textContent;label.textContent='Very long model name for testing';window.effortFixture=document.createElement('span');effortFixture.className='composer-menu-meta';effortFixture.textContent='Очень высокий';label.after(effortFixture)")
        assert js("const access=document.querySelector('.composer-access-menu').getBoundingClientRect(), model=document.querySelector('.composer-model-menu').getBoundingClientRect(), send=document.querySelector('.composer-submit').getBoundingClientRect();return access.right <= model.left && model.right <= send.left && send.right <= innerWidth && document.documentElement.scrollWidth <= innerWidth"), 'Composer controls overlap with a long model name'
        for menu in ['access', 'model']:
            js(f"document.querySelector('.composer-{menu}-menu summary').click()")
            wait_for(lambda: js(f"return document.querySelector('.composer-{menu}-menu').open"), 'Composer options did not open')
            assert js("const r=document.querySelector('.composer-menu[open] .composer-menu-panel').getBoundingClientRect(), shell=document.querySelector('.composer-shell').getBoundingClientRect();return r.left >= shell.left && r.right <= shell.right && r.top >= 0"), 'Composer options leave the input bounds'
            assert js("return [...document.querySelectorAll('.composer-menu[open] .menu-option-row')].every(button=>button.getBoundingClientRect().width > 150)"), 'Menu rows inherited the round send button style'
            if width == 390 and menu == 'model':
                command('/execute/async', {'script': 'const done=arguments[arguments.length-1];Promise.all(document.getAnimations().filter(a=>a.effect.getTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>done(null))', 'args': []})
                Path('/tmp/proteus-ui-composer-mobile.png').write_bytes(base64.b64decode(command('/screenshot', None)))
            js("window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
            wait_for(lambda: js(f"return !document.querySelector('.composer-menu[open]') && document.activeElement.matches('.composer-{menu}-menu summary')"), 'Escape did not close options and restore focus')
        js("document.querySelector('.composer-model-menu .composer-menu-model').textContent=window.originalModelLabel;window.effortFixture.remove()")
        draft('')

    command('/window/rect', {'width': 1440, 'height': 1000})
    js("if (document.querySelector('.app-layout.sidebar-collapsed'))[...document.querySelectorAll('[data-panel-toggle=sidebar]')].find(b=>!b.closest('[inert]')).click();if (!document.querySelector('.info-panel.open'))[...document.querySelectorAll('[data-panel-toggle=info]')].find(b=>!b.closest('[inert]')).click(); document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-choice=notes]')"), 'Settings failed after layout checks')
    wait_for(lambda: js("return document.querySelector('.settings-link').classList.contains('active')"), 'Settings tab indicator stale')
    command('/execute/async', {'script': 'const done=arguments[arguments.length-1];requestAnimationFrame(()=>requestAnimationFrame(()=>done(null)))', 'args': []})
    Path('/tmp/proteus-ui-settings.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("document.querySelector('.topnav a[href=\"/\"]').click()")
    wait_for(lambda: js("return document.querySelector('[data-extension-id=model-quota] .extension-panel-content')?.shadowRoot?.textContent.includes('73% осталось')"), 'Quota failed after layout checks')
