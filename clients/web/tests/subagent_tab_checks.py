"""Real Rust reducers/rendering through the existing EventSource fixture boundary."""
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin):
    command('/url', {'url': web + '/foundation.html?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return document.querySelector('.connection-badge')?.classList.contains('completed') && !!document.querySelector('.tab-workspace')"), 'Subagent fixture client did not connect')
    # Capture the authoritative session id from a real baseline, then inject
    # typed runtime events through the same deserialize/reducer path as SSE.
    js("""
      window.childFixtureSnapshot=null;
      document.querySelector('.connection-badge').click();
      fixtureSources.at(-1).addEventListener('output', event=>{
        const output=JSON.parse(event.data);
        if(output.type==='event' && output.event.type==='session_snapshot')window.childFixtureSnapshot=output.event.snapshot;
      });
    """)
    wait_for(lambda: js('return !!window.childFixtureSnapshot'), 'No authoritative baseline for child events')
    js("""
      window.childFixtureId=crypto.randomUUID();
      window.childFixtureRoot=childFixtureSnapshot.root_thread_id||crypto.randomUUID();
      window.childFixtureSeq=childFixtureSnapshot.seq+1;
      window.childEmit=(event,thread=childFixtureRoot)=>fixtureSources.at(-1).dispatchEvent(new MessageEvent('output',{data:JSON.stringify({
        type:'event',event:{type:'runtime',envelope:{schema_version:2,event_id:crypto.randomUUID(),session_id:childFixtureSnapshot.session_id,
          thread_id:thread,turn_id:null,seq:childFixtureSeq++,timestamp_ms:Date.now(),event}}
      })}));
      window.childCall=(id,name,args)=>({ToolCallRequested:{call:{id,name,args,surface:'function',raw_arguments:null}}});
      window.childResult=(id,text)=>({ToolFinished:{result:{call_id:id,ok:true,output:text,content:[],error:null,metadata:{}}}});
      childEmit(childCall('fixture-child-task','task',{agent_type:'reviewer',description:'Проверить вкладку субагента — задача fixture'}));
      childEmit({SubagentStarted:{role:'reviewer',description:'Проверить вкладку субагента — задача fixture',child_thread_id:childFixtureId}});
      childEmit(childCall('fixture-child-shell','shell',{command:'echo child-output-fixture'}),childFixtureId);
    """)
    wait_for(lambda: js("return document.querySelectorAll('.subagent-tab-link').length===1"), 'Child events did not render a compact row')
    assert js("return !document.querySelector('.results-panel .subagent-tool-list') && !document.querySelector('.results-panel .tool-preview')"), 'Child details filled the main chat'
    js("document.querySelector('.subagent-tab-link').click()")
    wait_for(lambda: js("return !!document.querySelector('.workspace-tab-content .subagent-tab-details .tool-card')"), 'Child workspace tab did not mount real tool cards')
    assert js("return document.querySelector('.workspace-tab-content .subagent-tab-details').textContent.includes('Проверить вкладку субагента — задача fixture')"), 'Task description was lost'
    js("window.childDetail=document.querySelector('.workspace-tab-content .subagent-tab-details');childDetail.querySelector('.tool-card-summary').click();childEmit(childResult('fixture-child-shell','Дочерний вывод: child-output-fixture'),childFixtureId)")
    wait_for(lambda: js("return childDetail.textContent.includes('Дочерний вывод: child-output-fixture')"), 'Live nested tool result did not update the tab')
    assert js("return getComputedStyle(childDetail.querySelector('.tool-card-summary')).display==='flex'"), 'Rust tool card styles are missing in owned tab'
    assert js("return !document.querySelector('.results-panel').textContent.includes('Дочерний вывод: child-output-fixture')"), 'Nested output leaked into the main chat'
    js("childEmit(childResult('fixture-child-task','Итог: child-summary-fixture'));childEmit({SubagentFinished:{role:'reviewer',status:'completed',iterations:3,child_thread_id:childFixtureId}})")
    wait_for(lambda: js("return childDetail.textContent.includes('Итог: child-summary-fixture') && document.querySelector('.subagent-tab-link').textContent.includes('готово')"), 'Child status/task outcome did not settle')
    assert js("return childDetail.scrollWidth<=childDetail.clientWidth+1"), 'Child details create horizontal overflow'
    js("window.childScrollProbe=document.createElement('div');childScrollProbe.style.height='1600px';childDetail.append(childScrollProbe);childDetail.scrollTop=childDetail.scrollHeight")
    assert js("return childDetail.scrollTop>0 && childDetail.clientHeight<=document.querySelector('.workspace-tab-content').clientHeight"), 'Long child activity is clipped instead of scrollable'
    js("childScrollProbe.remove();window.childToolCard=childDetail.querySelector('.tool-card');document.querySelector('.workspace-group[data-group=\"0\"] .workspace-transfer').click()")
    assert js("return childDetail===document.querySelector('.workspace-tab-content .subagent-tab-details') && childDetail.querySelector('.tool-card')===childToolCard && document.querySelector('.workspace-tab[data-owned=true][data-client=false]').closest('[data-group]').dataset.group==='1'"), 'Moving child tab remounted its Rust-owned tool content'
    js("document.querySelector('.workspace-group[data-group=\"1\"] .workspace-transfer').click()")
    assert js("return childDetail.querySelector('.tool-card')===childToolCard && childDetail.textContent.includes('Дочерний вывод: child-output-fixture')"), 'Returning child tab lost live output'
    js("document.querySelector('.workspace-tab[data-owned=true][data-client=false] .workspace-tab-close').click()")
    wait_for(lambda: js("return !document.querySelector('.workspace-tab[data-owned=true][data-client=false]') && !childDetail.querySelector('.tool-card')"), 'Closing child tab retained its tool subscriptions/content')
    assert js("return !!childDetail.closest('.subagent-detail-parking')"), 'Closed root was not returned to its client owner'
    js("document.querySelector('.subagent-tab-link').click()")
    wait_for(lambda: js("return document.querySelector('.workspace-tab-content .subagent-tab-details')===childDetail && !!childDetail.querySelector('.tool-card')"), 'Reopening remounted or lost the detail root')
    js("childDetail.querySelector('.tool-card-summary').click()")
    wait_for(lambda: js("return childDetail.textContent.includes('Дочерний вывод: child-output-fixture') && childDetail.textContent.includes('Итог: child-summary-fixture')"), 'Reopening lost tool output/task outcome')
    js("document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('.workspace-tab[data-owned=true][data-client=false]') && childDetail.isConnected"), 'Settings discarded the live child tab')
    js("window.childSession=new URL(location.href).searchParams.get('session_dir');window.childBoard=document.querySelector('.tab-workspace');document.querySelector('[aria-label=\"Новая сессия\"]').click()")
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')!==childSession && document.querySelector('.connection-badge').classList.contains('completed')"), 'New session did not connect after retained settings')
    assert js("return !document.querySelector('.workspace-tab[data-owned=true][data-client=false]') && !childDetail.isConnected && document.querySelector('.tab-workspace')===childBoard"), 'Session change retained stale child tab or replaced workspace'
    print('PASS: real child reducers; compact chat; owned detail tab; live output/status; cross-group root identity; close/reopen; retained navigation and session teardown', flush=True)
