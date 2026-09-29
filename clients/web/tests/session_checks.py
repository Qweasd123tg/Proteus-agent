"""Client-owned state on real streaming, reconnect and a loaded transcript."""
import json
from urllib.parse import urlencode
from message_nav_checks import run as check_message_nav

# Runs before the compiled client. Startup cases substitute selection/error responses;
# normal snapshots, commands and deltas use the agent with a fixture transcript prefix.
BOOTSTRAP = r'''<script>
const startupParams=new URL(location.href).searchParams;
const startupFixture=startupParams.get('startup_fixture');
window.startupCommands=[];
if(startupFixture){
  const stale='/tmp/proteus-unavailable-session-fixture';
  sessionStorage.setItem('proteus.selectedSessionDir:'+startupParams.get('server'),stale);
  const originalFetch=window.fetch;
  window.fetch=async(input,...args)=>{
    const path=new URL(input instanceof Request?input.url:input,location.href).pathname;
    if(path==='/resume'||path==='/new-session'){
      startupCommands.push({path,body:JSON.parse(await input.clone().text())});
      if(startupFixture==='resume-error')return new Response('startup-resume-fixture-error',{status:500});
    }
    if(path==='/sessions'&&startupFixture==='catalog-error')return new Response('startup-catalog-fixture-error',{status:500});
    const response=await originalFetch(input,...args);
    if(path==='/sessions'&&startupFixture==='live-bootstrap'){
      const catalog=await response.json();
      return new Response(JSON.stringify(catalog.filter(item=>item.session_dir!==startupParams.get('startup_bootstrap'))),{status:200,headers:{'content-type':'application/json'}});
    }
    if(path==='/bootstrap'){
      const bootstrap=await response.json();
      bootstrap.session_dir=startupFixture==='fresh'?null:startupParams.get('startup_bootstrap');
      return new Response(JSON.stringify(bootstrap),{status:200,headers:{'content-type':'application/json'}});
    }
    return response;
  };
}
const historyFixture = Array.from({length: 240}, (_,i)=>({
  message_id:'00000000-0000-0000-0000-'+i.toString(16).padStart(12,'0'), phase:null, role:i%2?'assistant':'user',
  text:'Сохранённое сообщение '+i+'. '+('Текст истории для проверки прокрутки. ').repeat(8),
  tool:null, subagent:null, streaming:false
}));
window.fixtureHistoryReads=0;
const OriginalEventSource=window.EventSource;
window.fixtureSources=[];
window.EventSource=class extends OriginalEventSource {
  constructor(...args){super(...args);this.outputHandlers=new Map();fixtureSources.push(this)}
  addEventListener(type,handler,...rest){
    if(type!=='output')return super.addEventListener(type,handler,...rest);
    const wrapped=event=>{
      const output=JSON.parse(event.data);
      if(output.type==='event' && output.event.type==='session_snapshot'){
        fixtureHistoryReads++;
        window.fixtureBaseItems ??= output.event.snapshot.transcript.length;
        output.event.snapshot.transcript=[...historyFixture,...output.event.snapshot.transcript.slice(fixtureBaseItems)];
        window.fixtureLastSnapshot=output.event.snapshot;
        event=new MessageEvent('output',{data:JSON.stringify(output)});
      }
      handler(event);
    };
    this.outputHandlers.set(handler,wrapped);
    return super.addEventListener(type,wrapped,...rest);
  }
  removeEventListener(type,handler,...rest){
    const wrapped=this.outputHandlers.get(handler)||handler;
    this.outputHandlers.delete(handler);
    return super.removeEventListener(type,wrapped,...rest);
  }
};
</script>'''


def check_inspector_startup(command, js, wait_for, web, origin):
    key = json.dumps('proteus.selectedSessionDir:' + origin)
    valid = js('return sessionStorage.getItem(' + key + ')')
    assert valid, 'Inspector startup fixture has no valid bootstrap target'
    for mode in ['stored', 'url', 'live-bootstrap', 'fresh', 'catalog-error', 'resume-error']:
        params = {'server': origin, 'token': 'extension-smoke', 'startup_fixture': mode, 'startup_bootstrap': valid}
        if mode == 'url':
            params['session_dir'] = '/tmp/proteus-unavailable-session-fixture'
        command('/url', {'url': web + '/inspector-foundation.html?' + urlencode(params)})
        if mode in ['catalog-error', 'resume-error']:
            wait_for(lambda: js("return document.querySelector('.empty-state-title')?.textContent.includes('Не удалось выбрать сессию')"), 'Inspector startup error did not surface')
            expected = 0 if mode == 'catalog-error' else 1
            assert js('return startupCommands.length') == expected, 'Inspector startup error opened a different session'
            assert js("return startupCommands.every(item=>item.path==='/resume')"), 'Inspector startup error created a new session'
            continue
        wait_for(lambda: js("return !!document.querySelector('.cfg-tabs button')"), 'Stale selection blocked Inspector startup')
        selected = js("return new URL(location.href).searchParams.get('session_dir')")
        assert selected and selected != '/tmp/proteus-unavailable-session-fixture', 'Inspector retained stale URL selection'
        assert js('return sessionStorage.getItem(' + key + ')') == selected, 'Inspector retained stale stored selection'
        if mode == 'fresh':
            assert selected != valid and js("return startupCommands.length===1 && startupCommands[0].path==='/new-session' && !startupCommands[0].body.source_session_dir"), 'Inspector did not create a fresh session'
        else:
            assert selected == valid and js('return startupCommands.length===1 && startupCommands[0].body.session_dir===' + json.dumps(valid)), 'Inspector stale selection overrode bootstrap'
        assert js("return new URL(document.querySelector('.inspector-chat-link').href).searchParams.get('session_dir')") == selected, 'Inspector chat link retained stale selection'
    js('sessionStorage.setItem(' + key + ',' + json.dumps(valid) + ')')
    print('PASS: Inspector stale stored/URL startup; live bootstrap; fresh session; strict catalog/resume errors', flush=True)


def run(command, js, wait_for, web, origin, loaded):
    # Exercise startup before the compiled client sees a stale tab selection.
    valid = js('return sessionStorage.getItem(' + json.dumps('proteus.selectedSessionDir:' + origin) + ')')
    assert valid, 'Session fixture has no valid bootstrap target'
    for mode in ['stored', 'url', 'live-bootstrap', 'fresh', 'catalog-error', 'resume-error']:
        params = {'server': origin, 'token': 'extension-smoke', 'startup_fixture': mode, 'startup_bootstrap': valid}
        if mode == 'url':
            params['session_dir'] = '/tmp/proteus-unavailable-session-fixture'
        command('/url', {'url': web + '/foundation.html?' + urlencode(params)})
        if mode in ['catalog-error', 'resume-error']:
            wait_for(lambda: js("return document.querySelector('.connection-badge')?.classList.contains('failed')"), 'Startup error did not surface')
            expected = 0 if mode == 'catalog-error' else 1
            assert js('return startupCommands.length') == expected, 'Startup error silently opened a different session'
            assert js("return startupCommands.every(item=>item.path==='/resume')"), 'Startup error created a new session'
            continue
        wait_for(lambda: js("return document.querySelector('.connection-badge')?.classList.contains('completed')"), 'Stale selection blocked startup')
        selected = js("return new URL(location.href).searchParams.get('session_dir')")
        assert selected and selected != '/tmp/proteus-unavailable-session-fixture', 'Stale selection remained in URL'
        assert js('return sessionStorage.getItem(' + json.dumps('proteus.selectedSessionDir:' + origin) + ')') == selected, 'Stale selection remained in storage'
        if mode == 'fresh':
            assert selected != valid and js("return startupCommands.length===1 && startupCommands[0].path==='/new-session' && !startupCommands[0].body.source_session_dir"), 'Unavailable selection did not create a fresh session'
        else:
            assert selected == valid and js('return startupCommands.length===1 && startupCommands[0].body.session_dir===' + json.dumps(valid)), 'Stale selection overrode server bootstrap'
    # Retain the original fixture session for streaming checks below.
    js('sessionStorage.setItem(' + json.dumps('proteus.selectedSessionDir:' + origin) + ',' + json.dumps(valid) + ')')
    print('PASS: stale stored/URL selection reconciled; fresh startup; catalog/resume errors stay strict', flush=True)
    command('/url', {'url': web + '/foundation.html?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return document.querySelector('.results-panel')?.textContent.includes('Сохранённое сообщение 239')"), 'Loaded transcript missing')
    wait_for(lambda: js("return document.querySelector('.connection-badge').classList.contains('completed')"), 'Foundation fixture did not connect')
    for _ in range(3):
        before_reads = js('return fixtureHistoryReads')
        js("document.querySelector('.connection-badge').click()")
        wait_for(lambda: js("return document.querySelector('.connection-badge').classList.contains('completed')"), 'Reconnect failed')
        wait_for(lambda: js('return fixtureHistoryReads') > before_reads, 'Manual reconnect did not resync history')
    assert js("return fixtureSources.length===4 && fixtureSources.slice(0,-1).every(s=>s.readyState===2 && s.outputHandlers.size===0 && !s.onopen && !s.onerror) && fixtureSources.at(-1).outputHandlers.size===1"), 'Reconnect retained old event handlers'
    check_message_nav(command, js, wait_for)
    js("window.oldCard=document.querySelector('.results-panel .task-card');window.oldMutations=0;window.oldObserver=new MutationObserver(records=>window.oldMutations+=records.length);oldObserver.observe(oldCard,{subtree:true,childList:true,characterData:true});const area=document.querySelector('.composer textarea');area.value='Проверь поток';area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Draft did not enable submit')
    js("document.querySelector('.composer-submit').click()")
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Абзац 3:') && !!document.querySelector('.composer-stop')"), 'Live streaming did not reach the UI')
    wait_for(lambda: js("const r=document.querySelector('.results-panel');return r.classList.contains('sticky-bottom') && r.scrollHeight-r.scrollTop-r.clientHeight<=4"), 'Streaming did not follow the bottom')
    js("const r=document.querySelector('.results-panel');r.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true}));r.scrollTop=600")
    wait_for(lambda: js("const r=document.querySelector('.results-panel'),y=r.getBoundingClientRect().top;return [...r.querySelectorAll('[data-transcript-row]')].some(n=>n.getBoundingClientRect().top<=y && n.getBoundingClientRect().bottom>y)"), 'Reading window missing')
    js("const r=document.querySelector('.results-panel'),y=r.getBoundingClientRect().top;window.readingAnchor=[...r.querySelectorAll('[data-transcript-row]')].find(n=>n.getBoundingClientRect().top<=y && n.getBoundingClientRect().bottom>y);window.readingTop=readingAnchor.getBoundingClientRect().top-y")
    wait_for(lambda: js("return !document.querySelector('.composer-stop')"), 'Stream failed to settle')
    assert js("return oldMutations===0"), 'Streaming changed an unchanged history card'
    assert js("return document.querySelectorAll('[data-transcript-row]').length<60"), 'History DOM grew with the transcript'
    assert js("return readingAnchor.isConnected && Math.abs(readingAnchor.getBoundingClientRect().top-document.querySelector('.results-panel').getBoundingClientRect().top-window.readingTop)<2"), 'Streaming pulled the reader away from history'
    js('oldObserver.disconnect()')
    print('PASS: loaded history; isolated card updates; live follow/reading; reconnect releases callbacks', flush=True)
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return document.querySelector('.results-panel')?.textContent.includes('Абзац 31:')"), 'Client did not restore normal API history')
    wait_for(lambda: js("return document.querySelector('[data-extension-id=model-quota] .extension-panel-content')?.shadowRoot?.textContent.includes('73% осталось')"), 'Quota did not restore after session checks')
