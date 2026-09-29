"""Session history/reconnect and late cancellation acknowledgement on a real server."""
import json


def run(command, js, wait_for, server):
    def send(text):
        js("const a=document.querySelector('.composer-input textarea');a.value=" + json.dumps(text) + ";a.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Send was not enabled')
        js("document.querySelector('.composer-submit').click()")

    # Count owned one-second clocks: visibility, not the focused route, owns the clock.
    js("window.clockTimers=new Set();window.originalInterval=window.setInterval;window.originalClearInterval=window.clearInterval;window.setInterval=(callback,delay,...args)=>{const id=originalInterval(callback,delay,...args);if(delay===1000)clockTimers.add(id);return id};window.clearInterval=id=>{clockTimers.delete(id);return originalClearInterval(id)}")
    before = server.model_requests
    server.stream_gate.clear()
    try:
        send('Проверка истории при переподключении')
        wait_for(lambda: server.model_requests > before, 'Streaming request did not start')
        wait_for(lambda: js("const c=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1);return c?.textContent.includes('Абзац 3:') && !c.textContent.includes('Абзац 31:')"), 'Expected partial stream')
        wait_for(lambda: js('return clockTimers.size===1'), 'Active chat must own exactly one activity timer')
        # WebKit smooth wheel starts within the bottom tolerance. That first
        # pixel must detach instead of allowing the next frame to snap back.
        js("const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:-1,bubbles:true}));window.onePixelTop=r.scrollHeight-r.clientHeight-1;r.scrollTop=onePixelTop")
        wait_for(lambda: js("return !document.querySelector('.results-panel').classList.contains('sticky-bottom') && Math.abs(document.querySelector('.results-panel').scrollTop-onePixelTop)<.5"), 'First pixel of upward scrolling snapped back to the bottom')
        js("window.pixelScrollRendered=false;requestAnimationFrame(()=>requestAnimationFrame(()=>window.pixelScrollRendered=true))")
        wait_for(lambda: js('return pixelScrollRendered'), 'Small upward scroll did not render')
        assert js("return !document.querySelector('.results-panel').classList.contains('sticky-bottom') && Math.abs(document.querySelector('.results-panel').scrollTop-onePixelTop)<.5"), 'Auto-scroll reclaimed a small upward gesture'
        js("const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:1,bubbles:true}));r.scrollTop=r.scrollHeight")
        wait_for(lambda: js("return document.querySelector('.results-panel').classList.contains('sticky-bottom')"), 'Scrolling down to the bottom did not resume follow')
        js("const a=document.querySelector('.composer textarea');a.value='Черновик во время ответа';a.dispatchEvent(new Event('input',{bubbles:true}));const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true}));r.scrollTop=120;window.readingScroll=r.scrollTop")
        js("document.querySelector('.settings-link').click()")
        wait_for(lambda: js('return clockTimers.size===0'), 'Leaving chat retained an activity timer')
        js("document.querySelector('.settings-back').click()")
        wait_for(lambda: js('return clockTimers.size===1'), 'Returning to chat did not restore exactly one activity timer')
        assert js("return document.querySelector('.composer textarea').value==='Черновик во время ответа'"), 'Settings navigation lost the draft during streaming'
        wait_for(lambda: js("return Math.abs(document.querySelector('.results-panel').scrollTop-readingScroll)<2"), 'Settings navigation lost the reading position during streaming')
        js("document.querySelector('[data-tab-id=\"client:chat\"]').closest('.workspace-group').querySelector('.workspace-transfer').click()")
        wait_for(lambda: js("return document.querySelectorAll('.workspace-group:not([hidden])').length===2 && clockTimers.size===1"), 'Splitting the chat duplicated or stopped its activity timer')
        js("document.querySelector('.settings-link').click()")
        wait_for(lambda: js("return document.querySelector('[data-client-workspace]').hidden && clockTimers.size===0"), 'Separate settings retained the hidden split chat timer')
        js("document.querySelector('.settings-back').click()")
        wait_for(lambda: js("return !document.querySelector('[data-client-workspace]').hidden && document.querySelectorAll('.workspace-group:not([hidden])').length===2 && clockTimers.size===1"), 'Returning from settings lost the split layout or its single timer')
        js("document.querySelector('[data-tab-id=\"client:chat\"] .workspace-tab-name').click();document.querySelector('[data-workspace-split]').click()")
        wait_for(lambda: js("return document.querySelectorAll('.workspace-group:not([hidden])').length===1 && clockTimers.size===1"), 'Merging groups duplicated or stopped the active chat timer')
        js("document.querySelector('.settings-link').click()")
        wait_for(lambda: js('return clockTimers.size===0'), 'Hiding merged chat retained its activity timer')
        js("document.querySelector('.settings-back').click()")
        wait_for(lambda: js('return clockTimers.size===1'), 'Returning to merged chat did not restore exactly one activity timer')
        js("document.querySelector('.connection-badge').click()")
        wait_for(lambda: js("const c=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1);return document.querySelector('.connection-badge').classList.contains('completed') && c?.textContent.includes('Абзац 0:') && c.textContent.includes('Абзац 3:') && !c.textContent.includes('Абзац 31:')"), 'Snapshot lost the already streamed prefix')
    finally:
        server.stream_gate.set()
    wait_for(lambda: js("const c=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1);return !document.querySelector('.composer-stop') && c?.textContent.includes('Абзац 31:')"), 'Reconnected stream did not settle')
    wait_for(lambda: js('return clockTimers.size===0'), 'Settled turn retained its activity timer')
    assert server.model_requests == before + 1, 'Reconnect repeated the model request'
    assert js("const t=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1).textContent;return (t.match(/Абзац 0:/g)||[]).length===1 && (t.match(/Абзац 31:/g)||[]).length===1"), 'Reconnect lost or duplicated streamed text'
    print('PASS: reconnect during streaming preserves the full response and does not repeat admission', flush=True)

    server.model_gate.clear()
    try:
        before = server.model_requests
        send('Первая отменяемая задача')
        wait_for(lambda: server.model_requests > before, 'First cancel request did not start')
        js("window.cancelReadSettled=false;const original=window.fetch;window.fetch=async(input,init)=>{if(!String(input.url||input).split('?')[0].endsWith('/cancel'))return original(input,init);window.fetch=original;const response=await original(input,init);return new Promise(resolve=>window.releaseCancelRead=()=>{resolve(response);requestAnimationFrame(()=>requestAnimationFrame(()=>window.cancelReadSettled=true))})};document.querySelector('.composer-stop').click()")
        wait_for(lambda: js("return typeof window.releaseCancelRead==='function' && !document.querySelector('.composer-stop')"), 'Cancellation did not settle independently of its HTTP acknowledgement')
        before = server.model_requests
        send('Вторая отменяемая задача')
        wait_for(lambda: server.model_requests > before, 'Second request did not start')
        wait_for(lambda: js("return !!document.querySelector('.composer-stop')"), 'Second run is not active')
        js('window.releaseCancelRead()')
        wait_for(lambda: js('return window.cancelReadSettled'), 'Cancel acknowledgement was not delivered')
        assert js("return !!document.querySelector('.composer-stop')"), 'Late cancellation acknowledgement cleared the new run'
        js("document.querySelector('.composer-stop').click()")
        wait_for(lambda: js("return !document.querySelector('.composer-stop')"), 'Second cancellation did not settle')
        print('PASS: confirmed cancellation; delayed cancel acknowledgement cannot clear a newer run', flush=True)
    finally:
        server.model_gate.set()

    js('window.setInterval=window.originalInterval;window.clearInterval=window.originalClearInterval')
