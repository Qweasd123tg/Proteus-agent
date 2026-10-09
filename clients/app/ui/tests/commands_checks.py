"""Slash completion and service/module/prompt dispatch against the real server."""


def run(command, js, wait_for, server):
    def draft(text):
        js("const input=document.querySelector('.composer-input textarea');input.value=" + repr(text) + ";input.dispatchEvent(new Event('input',{bubbles:true}));input.focus()")

    def submit(text):
        draft(text)
        wait_for(lambda: not js("return document.querySelector('.composer-submit').disabled"), 'Command submit unavailable')
        js("document.querySelector('.composer-submit').click()")

    def cleared():
        return js("return document.querySelector('.composer-input textarea').value==='' ")

    before = server.model_requests
    draft('/rev')
    wait_for(lambda: js("return document.querySelector('.slash-commands')?.textContent.includes('/review')"), 'Profile command missing from completion')
    js("document.querySelector('.composer-input textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',bubbles:true,cancelable:true}))")
    wait_for(lambda: js("return document.querySelector('.composer-input textarea').value==='/review '"), 'Tab did not complete command')
    draft('/')
    wait_for(lambda: js("return !!document.querySelector('.slash-commands')"), 'Command menu did not reopen')
    js("document.querySelector('.composer-input textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}))")
    assert js("return [...document.querySelectorAll('.slash-command')].findIndex(e=>e.classList.contains('selected'))") == 1
    js("document.querySelector('.composer-input textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}))")
    wait_for(lambda: not js("return !!document.querySelector('.slash-commands')"), 'Escape did not close completion')
    submit('/help')
    wait_for(cleared, '/help did not finish')
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('/dcp')"), 'Dynamic help missed module command')
    submit('/dcp stats')
    wait_for(cleared, '/dcp stats did not finish')
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('DCP Statistics')"), 'DCP stats output missing')
    submit('/mode plan')
    wait_for(cleared, '/mode did not finish')
    submit('/unknown-command')
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('unknown command')"), 'Unknown command was not rejected')
    assert js("return document.querySelector('.composer-input textarea').value") == '/unknown-command', 'Rejected command lost the draft'
    submit('/echo')
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('produced an empty message')"), 'Empty prompt expansion was not rejected')
    assert js("return document.querySelector('.composer-input textarea').value") == '/echo', 'Empty expansion lost the draft'
    assert server.model_requests == before, 'Service/module commands called the model'
    submit('/review target.rs')
    wait_for(lambda: server.model_requests == before + 1 and not js("return !!document.querySelector('.composer-stop')"), 'Prompt command did not run an ordinary turn')
    assert any('Review target.rs carefully' in str(item) for item in server.model_inputs[-1]['input']), 'Prompt was not expanded'
    submit('//literal slash')
    wait_for(lambda: server.model_requests == before + 2 and not js("return !!document.querySelector('.composer-stop')"), 'Escaped slash was not sent')
    assert any('/literal slash' in str(item) for item in server.model_inputs[-1]['input']), 'Literal slash was lost'
    print('PASS: shared slash catalog, keyboard completion, service/module no-model execution, retained errors, prompt turns and literal slash', flush=True)
