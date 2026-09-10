"""Client-owned state on real streaming, reconnect and a loaded transcript."""
from urllib.parse import urlencode
from message_nav_checks import run as check_message_nav

# Runs before the compiled client. Only the transcript prefix is substituted; snapshots, commands and deltas use the agent.
BOOTSTRAP = r'''<script>
const historyFixture = Array.from({length: 240}, (_,i)=>({
  message_id:'history-'+i, phase:null, role:i%2?'assistant':'user',
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


def run(command, js, wait_for, web, origin, loaded):
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
    wait_for(lambda: js("return !document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes('Абзац 31:')"), 'Stream failed to settle')
    assert js("return oldCard.isConnected && oldMutations===0"), 'Streaming replaced an unchanged history card'
    assert js("return Math.abs(document.querySelector('.results-panel').scrollTop-600)<2"), 'Streaming pulled the reader away from history'
    js('oldObserver.disconnect()')
    print('PASS: loaded history; isolated card updates; live follow/reading; reconnect releases callbacks', flush=True)
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return document.querySelector('.results-panel')?.textContent.includes('Абзац 31:')"), 'Client did not restore normal API history')
    wait_for(lambda: js("return document.querySelector('[data-extension-id=model-quota] .extension-panel-content')?.shadowRoot?.textContent.includes('73% осталось')"), 'Quota did not restore after session checks')
