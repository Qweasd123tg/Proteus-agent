"""Session history/reconnect and late cancellation acknowledgement on a real server."""
import json


def run(command, js, wait_for, server):
    def send(text):
        js("const a=document.querySelector('.composer-input textarea');a.value=" + json.dumps(text) + ";a.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Send was not enabled')
        js("document.querySelector('.composer-submit').click()")

    before = server.model_requests
    server.stream_gate.clear()
    try:
        send('Проверка истории при переподключении')
        wait_for(lambda: server.model_requests > before, 'Streaming request did not start')
        wait_for(lambda: js("const c=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1);return c?.textContent.includes('Абзац 3:') && !c.textContent.includes('Абзац 31:')"), 'Expected partial stream')
        js("document.querySelector('.connection-badge').click()")
        wait_for(lambda: js("const c=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1);return document.querySelector('.connection-badge').classList.contains('completed') && c?.textContent.includes('Абзац 0:') && c.textContent.includes('Абзац 3:') && !c.textContent.includes('Абзац 31:')"), 'Snapshot lost the already streamed prefix')
    finally:
        server.stream_gate.set()
    wait_for(lambda: js("const c=[...document.querySelectorAll('.results-panel .role-assistant')].at(-1);return !document.querySelector('.composer-stop') && c?.textContent.includes('Абзац 31:')"), 'Reconnected stream did not settle')
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
