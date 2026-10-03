"""Chat search finds messages outside the rendered window and paints them."""


def run(command, js, wait_for, server, capture):
    def send(text):
        js("const input=document.querySelector('.composer-input textarea');input.value=" + repr(text) + ";input.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Draft did not enable send')
        before = server.model_requests
        js("document.querySelector('.composer-submit').click()")
        wait_for(lambda: server.model_requests > before and not js("return !!document.querySelector('.composer-stop')"), 'Turn did not finish')

    def search(text):
        js("const input=document.querySelector('.chat-search input');input.value=" + repr(text) + ";input.dispatchEvent(new Event('input',{bubbles:true}))")

    def count():
        return js("return document.querySelector('.chat-search-count')?.textContent||''")

    def current_visible():
        return js("const h=CSS.highlights?.get('chat-search-current');if(!h||!h.size)return false;const r=[...h][0].getBoundingClientRect(),v=document.querySelector('.results-panel').getBoundingClientRect();return r.height>0&&r.top>=v.top&&r.bottom<=v.bottom")

    wait_for(lambda: not js("return !!document.querySelector('.composer-stop')"), 'Chat did not settle before the check')
    send('Найди маркер-один в начале')
    send('Второй длинный ответ')
    send('Третий длинный ответ')
    js("const r=document.querySelector('.results-panel');r.scrollTop=r.scrollHeight")
    assert not js("return [...document.querySelectorAll('[data-transcript-row]')].some(r=>r.textContent.includes('маркер-один'))"), 'The first prompt is still rendered; the check needs a virtualised chat'
    js("window.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyF',key:'f',ctrlKey:true,bubbles:true,cancelable:true}))")
    wait_for(lambda: js("return document.activeElement===document.querySelector('.chat-search input')"), 'Ctrl+F did not open chat search')
    search('маркер-один')
    wait_for(lambda: count() == '1 из 1', 'Search missed a message outside the window: ' + count())
    wait_for(current_visible, 'The match is not painted in view')
    search('Абзац 3:')
    wait_for(lambda: count().endswith('из 3') and count().startswith('3'), 'Repeated text is not counted per message: ' + count())
    first = js("return document.querySelector('[data-transcript-row]')&&[...CSS.highlights.get('chat-search-current')][0].startContainer.parentElement.closest('[data-transcript-row]').dataset.transcriptRow")
    js("document.querySelector('.chat-search input').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}))")
    wait_for(lambda: count() == '2 из 3', 'Enter did not step to the older match: ' + count())
    wait_for(lambda: current_visible() and js("return [...CSS.highlights.get('chat-search-current')][0].startContainer.parentElement.closest('[data-transcript-row]').dataset.transcriptRow") != first, 'Stepping did not move the painted match')
    capture('chat-search')
    search('несуществующая-строка')
    wait_for(lambda: count() == 'Нет совпадений', count())
    js("document.querySelector('.chat-search input').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}))")
    wait_for(lambda: not js("return !!document.querySelector('.chat-search')"), 'Escape did not close search')
    assert js("return !CSS.highlights.get('chat-search')?.size"), 'Closing left highlights'
    print('PASS: Ctrl+F searches the whole chat, steps between messages, paints the match in view and closes on Escape', flush=True)
