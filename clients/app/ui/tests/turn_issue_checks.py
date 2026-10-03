"""A turn that ends without an answer says why, and history keeps it."""


def run(command, js, wait_for, server, capture):
    def send(text):
        js("const input=document.querySelector('.composer-input textarea');input.value=" + repr(text) + ";input.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Draft did not enable send')
        js("document.querySelector('.composer-submit').click()")

    def settled():
        return not js("return !!document.querySelector('.composer-stop')")

    def issues():
        return js("return [...document.querySelectorAll('[data-turn-issue]')].map(c=>[c.className,c.querySelector('strong').textContent,c.querySelector('.turn-issue-detail')?.textContent||''])")

    wait_for(settled, 'Chat did not settle before the check')
    server.model_failure = (500, {'error': {'message': 'fixture overload', 'type': 'server_error'}})
    try:
        before = server.model_requests
        send('Сломайся')
        wait_for(lambda: server.model_requests > before and settled() and issues(), 'Failed turn did not explain itself')
    finally:
        server.model_failure = None
    print('Turn issues:', issues(), flush=True)
    assert issues() == [['turn-issue error', 'Ход завершился ошибкой', 'fixture overload']], issues()
    assert not js("return [...document.querySelectorAll('.role-system')].some(x=>x.textContent.includes('AppServer '))"), 'Raw failure text still shown'
    capture('turn-issue')
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')") and len(issues()) == 1, 'History lost the failed turn')

    # Stopping a turn is the user's own action: a quiet note, not an error.
    server.model_gate.clear()
    try:
        before = server.model_requests
        send('Начни и остановись')
        wait_for(lambda: server.model_requests > before and js("return !!document.querySelector('.composer-stop')"), 'Turn did not start')
        js("document.querySelector('.composer-stop').click()")
        wait_for(lambda: settled() and len(issues()) == 2, 'Canceled turn left no note')
    finally:
        server.model_gate.set()
    assert issues()[-1][:2] == ['turn-issue muted', 'Ход остановлен'], issues()
    print('PASS: failed turn shows the provider error as a callout that survives reload; a stopped turn is a quiet note', flush=True)
