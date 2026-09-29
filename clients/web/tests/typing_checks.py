"""The built renderer receives typed text deltas, retaining existing DOM."""
import json
from urllib.parse import urlencode
import threading


def run(command, js, wait_for, web, origin, server):
    command('/url', {'url': web + '/foundation.html?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return document.querySelector('.connection-badge')?.classList.contains('completed') && !!window.fixtureLastSnapshot && !!document.querySelector('.tab-workspace')"), 'Typing fixture did not receive its authoritative snapshot')
    js("""
      window.typingProbe={samples:[],started:null,marker:''};
      window.typingArrival=event=>{
        const output=JSON.parse(event.data);
        if(output.type==='event'&&output.event.type==='runtime'&&output.event.envelope.event.AssistantTextDelta)
          typingProbe.started=performance.now();
      };
      for(const source of fixtureSources)source.addEventListener('output',typingArrival);
      window.typingObserver=new MutationObserver(()=>{
        if(typingProbe.started===null||!typingProbe.marker)return;
        const active=document.querySelector('.streaming-message');
        if(active?.textContent.includes(typingProbe.marker)){
          typingProbe.samples.push(performance.now()-typingProbe.started);typingProbe.started=null;
        }
      });
      typingObserver.observe(document.querySelector('.results-panel'),{childList:true,characterData:true,subtree:true});
    """)
    metrics = []
    server.model_requests = 2
    server.typing_gate = threading.Semaphore(0)
    server.typing_completed = threading.Event()
    try:
        for label, text, selector in [
            ('short-prose', 'Короткий ответ. Уже готовый абзац.\n\nПродолжение', 'p:last-child'),
            ('long-code', 'Готовый абзац.\n\n```rust\n' + 'let value = "строка кода & text";\n' * 1200, 'pre code'),
            ('raw-html', '<details><summary>Детали</summary><p>Уже готовая часть</p></details>\n\n' + 'Продолжение ответа. ' * 800, 'p:last-child'),
        ]:
            chunks = [text] + [("\n// " if label == "long-code" else " ") + f"TYPINGPROBE{index}END" for index in range(12)]
            server.typing_chunks = chunks
            server.typing_completed.clear()
            js('typingProbe.samples=[];typingProbe.marker="";const input=document.querySelector(".composer textarea");input.value=' + json.dumps('Typing '+label) + ';input.dispatchEvent(new Event("input",{bubbles:true}))')
            wait_for(lambda: js('return !document.querySelector(".composer-submit").disabled'), 'Typing composer did not enable sending')
            js('document.querySelector(".composer-submit").click()')
            wait_for(lambda: js('return !!document.querySelector(".composer-stop")'), 'Typing turn did not start')
            server.typing_gate.release()
            wait_for(lambda: js("return !!document.querySelector('.streaming-message " + selector + "')"), 'Typing case did not render: ' + label)
            wait_for(lambda: js('return [...document.querySelectorAll(".streaming-message")].some(node=>node.textContent.includes(' + json.dumps(text.splitlines()[-1][-40:].strip()) + '))'), 'Initial typing text did not render completely: ' + label)
            js('window.typingNode=[...document.querySelectorAll(' + json.dumps('.streaming-message ' + selector) + ')].at(-1);window.typingTextNode=typingNode.firstChild;window.typingLosses=0;window.typingDetails=document.querySelector(".streaming-message details");if(typingDetails)typingDetails.open=true')
            for index in range(12):
                chunk = chunks[index + 1]
                js('typingProbe.marker=' + json.dumps(chunk.strip()))
                server.typing_gate.release()
                try:
                    wait_for(lambda: js('return [...document.querySelectorAll(".streaming-message")].some(node=>node.textContent.includes(typingProbe.marker))'), 'Typed suffix was not rendered: ' + label)
                except AssertionError:
                    print(js('return {marker:typingProbe.marker,samples:typingProbe.samples,rows:[...document.querySelectorAll(".results-panel .message")].map(node=>({class:node.className,text:node.textContent.slice(-300)}))}'), flush=True)
                    raise
                js('const current=[...document.querySelectorAll(' + json.dumps('.streaming-message ' + selector) + ')].at(-1);if(current!==typingNode||current.firstChild!==typingTextNode||!typingTextNode.textContent.includes(typingProbe.marker))typingLosses++')
            result = js("""
              const sorted=typingProbe.samples.slice().sort((a,b)=>a-b);
              return {samples:sorted.length,p50:sorted[Math.floor(sorted.length*.5)]||0,
                p95:sorted[Math.min(sorted.length-1,Math.floor(sorted.length*.95))]||0,
                lostNodes:typingLosses,details:!typingDetails||(typingDetails.isConnected&&typingDetails.open)};
            """)
            result['case'] = label
            metrics.append(result)
            server.typing_completed.set()
            wait_for(lambda: js('return !document.querySelector(".streaming-message")'), 'Completed typing case remained streaming')
            wait_for(lambda: js('return !document.querySelector(".composer-stop")'), 'Completed typing turn did not settle')
            assert js('return document.querySelector(".results-panel").textContent.includes("TYPINGPROBE11END")'), 'Completion lost the last text suffix'
    finally:
        server.typing_completed.set()
        for _ in range(20): server.typing_gate.release()
        js('for(const source of fixtureSources)source.removeEventListener("output",typingArrival);typingObserver.disconnect()')
    print('Typing delta-to-DOM milliseconds (not presented FPS): ' + json.dumps(metrics), flush=True)
    assert all(row['samples'] >= 10 for row in metrics), 'Typing samples were not collected'
    assert all(row['lostNodes'] == 0 for row in metrics), 'Streaming replaced existing text/element nodes'
    assert all(row['details'] for row in metrics), 'Streaming discarded opened details'
    print('PASS: short prose, long code and raw HTML retain text nodes, opened details and final suffix', flush=True)
