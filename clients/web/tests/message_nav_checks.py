"""The real navigation rail follows visible turns, hover, keyboard and input height."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    wait_for(lambda: js("return document.querySelectorAll('.msg-nav-tick').length===120"), 'History navigation is incomplete')
    js("const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true}));r.scrollTop=3200")

    def visibility_matches():
        return js("""
            const r=document.querySelector('.results-panel').getBoundingClientRect();
            const bottom=document.querySelector('.composer').getBoundingClientRect().top;
            const ticks=[...document.querySelectorAll('.msg-nav-tick')];
            const expected=ticks.filter(t=>{
                const first=document.getElementById(t.dataset.messageId); let last=first;
                while(last.nextElementSibling && !last.nextElementSibling.matches('.user-turn')) last=last.nextElementSibling;
                return first.getBoundingClientRect().top<bottom && last.getBoundingClientRect().bottom>r.top;
            });
            const actual=ticks.filter(t=>t.classList.contains('is-visible'));
            return expected.length>1 && expected.length===actual.length && expected.every(t=>actual.includes(t));
        """)
    wait_for(visibility_matches, 'Rail does not highlight all visible turns above the input')
    assert js("return document.querySelector('.msg-nav-track').getBoundingClientRect().height<=280"), 'Long navigation does not fit the viewport'
    js("const tick=document.querySelectorAll('.msg-nav-tick')[50], r=tick.getBoundingClientRect();tick.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:r.left+10,clientY:r.top+r.height/2}))")
    wait_for(lambda: js("return document.querySelector('.msg-nav.preview-open .msg-nav-preview')?.textContent.includes('Сохранённое сообщение 100')"), 'Hovered message preview is wrong')
    if js("return matchMedia('(prefers-reduced-motion: reduce)').matches"):
        assert js("return [...document.querySelectorAll('.msg-nav-tick')].every(t=>!t.style.getPropertyValue('--tick-wave'))"), 'Navigation wave ignores reduced motion'
    else:
        assert js("const t=document.querySelectorAll('.msg-nav-tick');return +t[50].style.getPropertyValue('--tick-wave') > +t[51].style.getPropertyValue('--tick-wave') && +t[51].style.getPropertyValue('--tick-wave') > +t[53].style.getPropertyValue('--tick-wave')"), 'Hover wave does not taper across neighbors'
    command('/execute/async', {'script': 'const done=arguments[arguments.length-1];requestAnimationFrame(()=>requestAnimationFrame(()=>Promise.all(document.querySelector(".msg-nav").getAnimations({subtree:true}).map(a=>a.finished.catch(()=>{}))).then(()=>done(null))))', 'args': []})
    Path('/tmp/proteus-ui-message-nav.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("const t=document.querySelectorAll('.msg-nav-tick')[50];t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true}))")
    assert js("return document.activeElement===document.querySelectorAll('.msg-nav-tick')[51]"), 'Arrow key did not select the next message'
    js('document.activeElement.click()')
    wait_for(lambda: js("const tick=document.activeElement, card=document.getElementById(tick.dataset.messageId), r=document.querySelector('.results-panel');return Math.abs(card.getBoundingClientRect().top-r.getBoundingClientRect().top)<2"), 'Navigation did not jump to the selected message')
    wait_for(visibility_matches, 'Visibility stayed stale after message jump')
    js("document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))")
    assert js("return !document.querySelector('.msg-nav').classList.contains('preview-open')"), 'Escape did not hide the preview'
    js("const area=document.querySelector('.composer textarea');area.value=('Long draft\\n').repeat(30);area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return document.querySelector('.composer textarea').clientHeight>150"), 'Input did not grow for navigation check')
    wait_for(visibility_matches, 'Rail highlights messages covered by the grown input')
    js("const area=document.querySelector('.composer textarea');area.value='';area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return document.querySelector('.composer textarea').clientHeight<80"), 'Input did not shrink after navigation check')
    js("document.querySelector('.jump-to-bottom').click()")
    wait_for(lambda: js("return document.querySelector('.results-panel').classList.contains('sticky-bottom')"), 'Navigation check did not restore follow mode')
    print('PASS: visible turn markers; compact rail; hover wave/preview; keyboard jump; composer occlusion', flush=True)
