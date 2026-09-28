"""Composer clearance survives reactive chat-width changes, without dock resizing."""
def run(command, js, wait_for):
    js("window.chatWidthBefore=document.querySelector('.session-workspace').style.getPropertyValue('--chat-max-width');window.dockBefore=document.querySelector('.composer').getBoundingClientRect().height; const h=document.querySelector('.chat-resize-handle');h.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,clientX:600}));document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:620}));document.querySelector('.app-layout').dispatchEvent(new MouseEvent('mouseup',{bubbles:true,clientX:620}));")
    wait_for(lambda: js("return document.querySelector('.session-workspace').style.getPropertyValue('--chat-max-width')!==chatWidthBefore"), 'Chat resize was not exercised')
    wait_for(lambda: js("return document.querySelector('.session-workspace').style.getPropertyValue('--composer-inset')!==''"), 'Chat resize erased composer inset: last lines are hidden behind input')
    js("const r=document.querySelector('.results-panel');window.scrollProbe=document.createElement('div');scrollProbe.innerHTML='<div style=height:1500px></div><p id=last-line-probe>Последняя строка</p>';r.append(scrollProbe);r.scrollTop=r.scrollHeight")
    assert js("return document.querySelector('#last-line-probe').getBoundingClientRect().bottom <= document.querySelector('.composer').getBoundingClientRect().top-16"), 'Last line cannot scroll above composer'
    assert js("return getComputedStyle(document.querySelector('.results-panel')).maskImage.includes('gradient')"), 'Chat fade disappeared'
    js("scrollProbe.remove()")
    print('PASS: composer clearance and fade after chat resize', flush=True)
