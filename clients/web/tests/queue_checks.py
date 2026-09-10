"""Queue UI against a real agent paused at its provider request."""
import base64
import json
from pathlib import Path


def run(command, js, wait_for, server):
    def input_text(value):
        js("const a=document.querySelector('.composer-input textarea');a.value=" + json.dumps(value) + ";a.dispatchEvent(new Event('input',{bubbles:true}))")

    def send(value):
        input_text(value)
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Queue draft did not enable submit')
        js("document.querySelector('.composer-submit').click()")

    server.model_gate.clear()
    try:
        before_requests = server.model_requests
        send('Проверка редактирования очереди')
        wait_for(lambda: js("return !!document.querySelector('.composer-stop')"), 'Queue fixture did not start')
        wait_for(lambda: server.model_requests > before_requests, 'Queue fixture did not reach the provider boundary')
        send('Первое ожидающее сообщение')
        wait_for(lambda: js("return document.querySelectorAll('.queued-prompt-row').length===1"), 'First queued message missing')
        send('Удаляемое ожидающее сообщение')
        wait_for(lambda: js("return document.querySelectorAll('.queued-prompt-row').length===2"), 'Second queued message missing')
        # Capture /pending before edits, then deliver it after newer SSE snapshots.
        js("window.pendingReadSettled=false;const original=window.fetch;window.fetch=async(input,init)=>{if(!String(input.url||input).split('?')[0].endsWith('/pending'))return original(input,init);window.fetch=original;const response=await original(input,init);return new Promise(resolve=>window.releasePendingRead=()=>{resolve(response);requestAnimationFrame(()=>requestAnimationFrame(()=>window.pendingReadSettled=true))})};document.querySelector('.connection-badge').click()")
        wait_for(lambda: js("return typeof window.releasePendingRead==='function'"), 'Pending read was not captured on reconnect')
        assert js("const q=document.querySelector('.composer-queue').getBoundingClientRect(), shell=document.querySelector('.composer-shell').getBoundingClientRect();return q.top<shell.top && q.left>shell.left && q.right<shell.right && !document.querySelector('.results-panel .queued-prompt-row')"), 'Queue is not attached above the input'
        input_text('Основной черновик сохранён')
        js("document.querySelector('.queued-prompt-row [aria-label=\"Редактировать сообщение\"]').click()")
        wait_for(lambda: js("return document.activeElement.matches('.queued-prompt-editor textarea')"), 'Queue editor did not receive focus')
        js("const a=document.querySelector('.queued-prompt-editor textarea');a.value='Отредактированное уточнение';a.dispatchEvent(new Event('input',{bubbles:true}))")
        js("document.querySelector('.queue-editor-actions .primary').click()")
        wait_for(lambda: js("return !document.querySelector('.queued-prompt-editor') && document.querySelector('.queued-prompt-text').textContent==='Отредактированное уточнение'"), 'Edited queue text did not reach the event stream')
        assert js("return document.querySelector('.composer-input textarea').value==='Основной черновик сохранён'"), 'Queue edit overwrote the main draft'
        # A rejected delete must leave the row present and expose an inline error.
        js("const original=window.fetch;window.fetch=(input,init)=>{if(!String(input.url||input).includes('/queue/delete'))return original(input,init);window.fetch=original;return Promise.resolve(new Response(JSON.stringify({type:'response',id:null,ok:false,output:null,error:'fixture rejection'}),{headers:{'Content-Type':'application/json'}}))}")
        js("document.querySelectorAll('.queued-prompt-row .queue-delete')[1].click()")
        wait_for(lambda: js("return !!document.querySelector('.queue-error')"), 'Rejected queue mutation did not show its error')
        assert js("return document.querySelectorAll('.queued-prompt-row').length===2"), 'Rejected deletion removed the message locally'
        js("document.querySelector('.queue-error button').click();document.querySelectorAll('.queued-prompt-row .queue-delete')[1].click()")
        wait_for(lambda: js("return document.querySelectorAll('.queued-prompt-row').length===1"), 'Queued message was not removed')
        js("window.releasePendingRead()")
        wait_for(lambda: js("return window.pendingReadSettled"), 'Delayed pending response was not released')
        assert js("return document.querySelectorAll('.queued-prompt-row').length===1 && document.querySelector('.queued-prompt-text').textContent==='Отредактированное уточнение'"), 'Delayed pending snapshot restored deleted or superseded queue state'
        Path('/tmp/proteus-ui-message-queue.png').write_bytes(base64.b64decode(command('/screenshot', None)))
        command('/refresh', {})
        wait_for(lambda: js("return document.querySelector('.queued-prompt-text')?.textContent==='Отредактированное уточнение'"), 'Pending snapshot did not preserve the edited queue after reload')
        wait_for(lambda: js("return !!document.querySelector('.composer-stop')"), 'Reload lost the active run behind the queue')
        assert js("return document.querySelectorAll('.queued-prompt-row').length===1"), 'Deleted queue message reappeared on reload'
        js("document.querySelector('.queued-prompt-row [aria-label=\"Редактировать сообщение\"]').click()")
        wait_for(lambda: js("return !!document.querySelector('.queued-prompt-editor textarea')"), 'Second editor did not open')
        js("const a=document.querySelector('.queued-prompt-editor textarea');a.value='Несохранённая правка';a.dispatchEvent(new Event('input',{bubbles:true}))")
        server.model_gate.set()
        wait_for(lambda: js("return !document.querySelector('.queued-prompt-row') && !!document.querySelector('.queue-notice')"), 'Editor did not report delivery during editing')
        assert js("return document.querySelector('.queued-prompt-editor textarea').value==='Несохранённая правка' && document.querySelector('.queue-editor-actions .primary').disabled"), 'Delivery lost the unsaved edit or kept a stale save action'
        js("document.querySelector('.queue-editor-actions .secondary').click()")
        wait_for(lambda: js("return !document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes('Отредактированное уточнение')"), 'Edited message did not settle into history')
        assert js("const r=document.querySelector('.results-panel').textContent;return !r.includes('Удаляемое ожидающее сообщение') && !r.includes('Первое ожидающее сообщение')"), 'Queue history contains a deleted or superseded message'
        print('PASS: queued rows above input; edit/delete; rejected mutation; delayed snapshot; reload snapshot; edit/delivery race; final history', flush=True)
    finally:
        server.model_gate.set()
