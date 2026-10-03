"""A chat that finishes in the background notifies; the click opens it."""


def run(command, js, wait_for, server):
    def selected():
        return js("return new URL(location.href).searchParams.get('session_dir')")

    def connected():
        return js("return document.querySelector('.connection-badge')?.classList.contains('completed')")

    def send(text):
        js("const input=document.querySelector('.composer-input textarea');input.value=" + repr(text) + ";input.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Draft did not enable send')
        js("document.querySelector('.composer-submit').click()")

    # Earlier steps may still be finishing a turn; that would be a real notice.
    first = selected()
    wait_for(lambda: not js("return !!document.querySelector('.composer-stop')") and 'работает' not in (js("return [...document.querySelectorAll('.session-item-shell')].find(r=>r.dataset.sessionDir===" + repr(first) + ")?.dataset.hoverDetail") or ''), 'Chat did not settle before the check')
    # The browser grants nothing in automation: record what would be shown.
    js("window.notices=[];window.Notification=class{static permission='granted';constructor(title,options){Object.assign(this,{title,...options});notices.push(this)}close(){}};document.hasFocus=()=>false")
    server.model_gate.clear()
    try:
        before = server.model_requests
        send('Фоновая задача')
        wait_for(lambda: server.model_requests > before, 'Model did not start')
        js("document.querySelector('[aria-label=\"Новая сессия\"]').click()")
        wait_for(lambda: selected() not in (None, first) and connected(), 'Second chat did not open')
        assert js("return notices.length") == 0, js("return JSON.stringify(notices.map(n=>[n.title,n.body,n.tag]))") + ' first=' + first
    finally:
        server.model_gate.set()
    wait_for(lambda: js("return notices.length===1"), 'Finished background chat did not notify')
    assert js("return notices[0].body") == 'Агент закончил работу.', js("return notices[0].body")
    assert js("return notices[0].tag") == first
    title = js("return [...document.querySelectorAll('.session-item-shell')].find(r=>r.dataset.sessionDir===" + repr(first) + ")?.dataset.hoverTitle")
    assert title and js("return notices[0].title") == title, (js("return notices[0].title"), title)
    js("notices[0].onclick()")
    wait_for(lambda: selected() == first and connected(), 'Notification click did not open its chat')
    # A focused window already shows the result.
    js("document.hasFocus=()=>true")
    before = server.model_requests
    send('Ещё один ход')
    wait_for(lambda: server.model_requests > before and not js("return !!document.querySelector('.composer-stop')"), 'Second turn did not finish')
    assert js("return notices.length") == 1, 'A focused window was notified'
    print('PASS: background chat finish notifies once with its title; click opens the chat; focused window stays quiet', flush=True)
