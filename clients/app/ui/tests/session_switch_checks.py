"""Session transitions while a real response or a resume request is pending."""
import json


def run(command, js, wait_for, server, origin):
    def selected():
        return js("return new URL(location.href).searchParams.get('session_dir')")

    def connected():
        return js("return document.querySelector('.connection-badge')?.classList.contains('completed')")

    def open_session(session):
        row = "[...document.querySelectorAll('.session-item-shell')].find(r=>r.dataset.sessionDir===" + json.dumps(session) + ")"
        wait_for(lambda: js('return !!' + row), 'Session missing from sidebar')
        js(row + ".querySelector('.session-item').click()")

    def draft(text):
        js("const a=document.querySelector('.composer textarea');a.value=" + json.dumps(text) + ";a.dispatchEvent(new Event('input',{bubbles:true}))")

    original = selected()
    js("document.querySelector('[aria-label=\"Новая сессия\"]').click()")
    wait_for(lambda: selected() != original and connected(), 'Second session did not open')
    other = selected()
    open_session(original)
    wait_for(lambda: selected() == original and connected(), 'Original session did not reopen')
    js("const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true}));r.scrollTop=500")
    wait_for(lambda: js("return !document.querySelector('.results-panel').classList.contains('sticky-bottom')"), 'Original session did not enter reading mode')
    open_session(other)
    wait_for(lambda: selected() == other and connected() and js("return document.querySelector('.results-panel').classList.contains('sticky-bottom')"), 'New chat inherited detached reading mode')
    open_session(original)
    wait_for(lambda: selected() == original and connected() and js("const r=document.querySelector('.results-panel');return r.classList.contains('sticky-bottom') && r.scrollHeight-r.clientHeight-r.scrollTop<2"), 'Long reopened chat did not start at its latest message')
    errors = []

    # Delay browser delivery of the delta flush and the next resume response.
    # The provider, event stream, and sidebar actions remain real.
    js("""
      window.switchFrames=[];window.switchRAF=window.requestAnimationFrame;
      window.requestAnimationFrame=callback=>callback.proteusStreamFlush
        ? (switchFrames.push(callback),0)
        : switchRAF(callback);
      window.switchFetch=window.fetch;
      window.fetch=(input,...args)=>new URL(input.url||input,location.href).pathname==='/resume'
        ? new Promise(resolve=>window.releaseSwitchResume=()=>resolve(switchFetch(input,...args)))
        : switchFetch(input,...args);
    """)
    server.stream_gate.clear()
    try:
        before = server.model_requests
        draft('Поток перед переключением чата')
        js("document.querySelector('.composer-submit').click()")
        wait_for(lambda: server.model_requests > before and js('return switchFrames.length>0'), 'Stream delta was not buffered')
        open_session(other)
        wait_for(lambda: selected() == other and js("return typeof releaseSwitchResume==='function'"), 'Pending session switch did not start')
        js('window.requestAnimationFrame=switchRAF;switchFrames.splice(0).forEach(callback=>callback())')
        # Cross a render boundary before inspecting the new transcript.
        js('window.switchRendered=false;requestAnimationFrame(()=>requestAnimationFrame(()=>window.switchRendered=true))')
        wait_for(lambda: js('return switchRendered'), 'Session switch did not render')
        if js("return document.querySelector('.results-panel').textContent.includes('Абзац')"):
            errors.append('Buffered response from the previous session appeared in the selected chat')
        js('window.fetch=switchFetch;releaseSwitchResume()')
        wait_for(connected, 'Target session did not connect')
    finally:
        server.stream_gate.set()
        js('window.requestAnimationFrame=switchRAF;window.fetch=switchFetch')

    open_session(original)
    wait_for(lambda: selected() == original and connected() and js("return !document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes('Абзац 31:')"), 'Original response did not settle after returning')
    draft('Черновик исходного чата')
    js("""
      window.failedSwitch=false;window.switchFetch=window.fetch;
      window.fetch=(input,...args)=>{
        if(new URL(input.url||input,location.href).pathname==='/resume'){
          window.failedSwitch=true;
          return Promise.resolve(new Response('session-switch-fixture-error',{status:503}));
        }
        return switchFetch(input,...args);
      };
    """)
    try:
        open_session(other)
        wait_for(lambda: js('return failedSwitch'), 'Failed resume was not requested')
        wait_for(lambda: (selected() == original and connected()) or js("return document.querySelector('.connection-badge')?.classList.contains('failed')"), 'Failed resume did not settle')
        if selected() != original or not connected():
            errors.append('Failed resume did not restore the previous chat connection')
        else:
            wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Абзац 31:')"), 'Failed resume lost original history')
            assert js("return document.querySelector('.composer textarea').value") == 'Черновик исходного чата', 'Failed resume lost the original draft'
            assert js('return sessionStorage.getItem(' + json.dumps('proteus.selectedSessionDir:' + origin) + ')') == original, 'Failed resume left stale stored selection'
            assert js("return document.querySelector('.toast-stack').textContent.includes('session-switch-fixture-error')"), 'Resume failure disappeared after recovery'
    finally:
        js('window.fetch=switchFetch')
    assert not errors, '; '.join(errors)
    # A pending failed selection must not roll back a later successful choice.
    js("""
      window.switchFetch=window.fetch;
      window.fetch=(input,...args)=>{
        if(new URL(input.url||input,location.href).pathname==='/resume'){
          window.fetch=switchFetch;
          return new Promise(resolve=>window.releaseStaleResume=()=>{
            resolve(new Response('obsolete-resume-fixture-error',{status:503}));
            requestAnimationFrame(()=>requestAnimationFrame(()=>window.staleResumeRendered=true));
          });
        }
        return switchFetch(input,...args);
      };
    """)
    open_session(other)
    wait_for(lambda: js("return typeof releaseStaleResume==='function'"), 'Delayed resume was not requested')
    js("document.querySelector('[aria-label=\"Новая сессия\"]').click()")
    wait_for(lambda: selected() not in (original, other, None) and connected(), 'Newer session did not connect')
    newest = selected()
    draft('Черновик нового чата')
    js('releaseStaleResume()')
    wait_for(lambda: js('return !!window.staleResumeRendered'), 'Delayed failure was not delivered')
    assert selected() == newest and connected(), 'Late failed resume replaced the newer session'
    assert js("return document.querySelector('.composer textarea').value") == 'Черновик нового чата', 'Late failed resume replaced the newer draft'
    assert js("return !document.querySelector('.toast-stack').textContent.includes('obsolete-resume-fixture-error')"), 'Stale resume error leaked into the newer session'
    open_session(original)
    wait_for(lambda: selected() == original and connected(), 'Original session did not restore after stale resume check')
    # Neither of two pending selections is a valid rollback target.
    js("""
      window.switchFetch=window.fetch;window.switchResumeCount=0;
      window.fetch=(input,...args)=>{
        if(new URL(input.url||input,location.href).pathname==='/resume'){
          if(++switchResumeCount===1)return new Promise(resolve=>window.releaseUnconfirmedResume=()=>
            resolve(new Response('superseded-resume-fixture-error',{status:503})));
          return Promise.resolve(new Response('latest-resume-fixture-error',{status:503}));
        }
        return switchFetch(input,...args);
      };
    """)
    try:
        open_session(other)
        wait_for(lambda: js("return typeof releaseUnconfirmedResume==='function'"), 'First pending selection did not start')
        open_session(newest)
        wait_for(lambda: js("return document.querySelector('.toast-stack').textContent.includes('latest-resume-fixture-error')") and connected(), 'Latest failed selection did not recover')
        assert selected() == original, 'Two pending switches rolled back to an unconfirmed session'
        wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Абзац 31:')"), 'Rapid failed selections lost original history')
        assert js("return document.querySelector('.composer textarea').value") == 'Черновик исходного чата', 'Rapid failed selections lost original draft'
    finally:
        js('window.fetch=switchFetch;releaseUnconfirmedResume()')
    print('PASS: buffered stream cannot cross sessions; failed resume restores connection, history and draft with a visible error', flush=True)
