"""Analysis UX on real usage data, then controlled error/unfinished request projections."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    root = "document.querySelector('.usage-details-host > div')?.shadowRoot"
    assert js("return document.querySelector('.analysis-title h1').textContent.length > 0"), 'Session title missing'
    assert js("return document.querySelector('.analysis-open-chat').textContent==='Открыть диалог'"), 'Return to selected chat missing'
    assert js("return new URL(location.href).searchParams.get('analysis_session')===document.querySelector('#analysis-session').value"), 'Analysis selection not addressed in URL'
    # Context has a distinct scope. Switching tabs preserves the report selection/details.
    js(f"const root={root};root.querySelector('.usage-request').open=true;document.querySelectorAll('.analysis-tabs button')[1].click()")
    wait_for(lambda: js("return !!document.querySelector('.analysis-context .context-map-scroll')"), 'Context snapshot did not load')
    assert js("return document.querySelector('.analysis-context').textContent.includes('Выбор хода в отчёте расхода не меняет этот снимок')"), 'Current context is confused with historical request scope'
    command('/refresh', {})
    wait_for(lambda: js("return !!document.querySelector('.analysis-context .context-map-scroll')"), 'Selected context tab was lost after reload')
    js("document.querySelectorAll('.analysis-tabs button')[0].click()")
    wait_for(lambda: js(f"return {root}?.querySelectorAll('.usage-request').length===2"), 'Request report was lost after tab change')

    # Retain the real response and add deterministic diagnostic cases through the same reader.
    js("""window.analysisFetch=window.fetch;window.fetch=async(input,init)=>{
      const response=await window.analysisFetch(input,init);
      if(String(input.url||input).includes('/usage')) {
        const snapshot=await response.clone().json(); window.analysisSource=snapshot;
        if(window.analysisFixture) return new Response(JSON.stringify(window.analysisFixture),{status:200,headers:{'Content-Type':'application/json'}});
      }
      return response;
    };""")
    js(f"{root}.querySelector('.usage-footer button').click()")
    wait_for(lambda: js("return !!window.analysisSource"), 'Real journal snapshot was not captured')
    js("""const snapshot=structuredClone(window.analysisSource), base=snapshot.requests[0];
      snapshot.revision+=1000;
      snapshot.requests.push(
        {...base,exchange_id:'diagnostic-error',status:'error',usage:null,finished_at_ms:base.started_at_ms+2000},
        {...base,exchange_id:'diagnostic-summary',origin:'compactor',turn_id:'diagnostic-turn',finished_at_ms:base.started_at_ms+9000},
        {...base,exchange_id:'diagnostic-unfinished',status:'unfinished',turn_id:null,usage:null,finished_at_ms:null});
      snapshot.latest_turn_id='diagnostic-turn';window.analysisFixture=snapshot;""")
    js(f"{root}.querySelector('.usage-footer button').click()")
    wait_for(lambda: js(f"return {root}.querySelectorAll('.usage-request').length===5"), 'Diagnostic fixture did not render')
    js(f"const root={root};window.analysisTotal=root.querySelector('.usage-headline > strong').textContent;const select=root.querySelector('[aria-label=\"Статус запроса\"]');select.value='problems';select.dispatchEvent(new Event('change'))")
    assert js(f"const root={root};return root.querySelectorAll('.usage-request').length===2 && root.querySelector('.usage-headline > strong').textContent===window.analysisTotal && root.querySelector('.usage-list-heading').textContent.includes('2 из 5')"), 'Problem filter changed the period total or omitted unfinished requests'
    js(f"const root={root};const search=root.querySelector('input');search.value='diagnostic-error';search.dispatchEvent(new Event('input'))")
    assert js(f"return {root}.querySelectorAll('.usage-request').length===1 && {root}.querySelector('.usage-request').dataset.exchangeId==='diagnostic-error'"), 'Search and status filters did not compose'
    js(f"const root={root};const search=root.querySelector('input');search.value='no-such-request';search.dispatchEvent(new Event('input'))")
    assert js(f"const root={root};return root.textContent.includes('По этим фильтрам запросов нет') && root.querySelector('.usage-list-heading button').disabled"), 'Empty filtered result is misleading or export stays enabled'
    js(f"const root={root};root.querySelector('.usage-filters button').click();const order=root.querySelector('[aria-label=\"Порядок запросов\"]');order.value='duration';order.dispatchEvent(new Event('change'))")
    assert js(f"const rows=[...{root}.querySelectorAll('.usage-request')];return rows[0].dataset.exchangeId==='diagnostic-summary' && rows.at(-1).dataset.exchangeId==='diagnostic-unfinished'"), 'Duration sort hides slow request or treats unfinished as zero'
    js(f"const root={root};const select=root.querySelector('[aria-label=\"Период расхода\"]');select.value='latest';select.dispatchEvent(new Event('change'));root.querySelector('.usage-request').open=true;root.querySelector('.usage-footer button').click()")
    wait_for(lambda: js(f"return !{root}.querySelector('.usage-footer button').disabled"), 'Report refresh did not finish')
    assert js(f"const root={root};return root.querySelectorAll('.usage-request').length===1 && root.querySelector('.usage-request').open && root.textContent.includes('Ход 2') && root.textContent.includes('Сжатие контекста')"), 'Refresh lost turn filter/details or stable turn identity'
    # Inspect the exact downloadable JSON without creating a file or invoking a download dialog.
    js("""window.analysisCreateURL=URL.createObjectURL;URL.createObjectURL=blob=>{window.analysisExport=blob;return window.analysisCreateURL(blob)};
      window.analysisAnchorClick=HTMLAnchorElement.prototype.click;HTMLAnchorElement.prototype.click=function(){if(!this.download)window.analysisAnchorClick.call(this)};""")
    js(f"{root}.querySelector('.usage-list-heading button').click();window.analysisExport.text().then(value=>window.analysisExportJSON=JSON.parse(value))")
    wait_for(lambda: js("return !!window.analysisExportJSON"), 'JSON export was not created')
    assert js("return window.analysisExportJSON.session_id===window.analysisSource.session_id && window.analysisExportJSON.requests.length===1 && window.analysisExportJSON.requests[0].exchange_id==='diagnostic-summary' && window.analysisExportJSON.filters.scope==='latest'"), 'Export does not identify the selected session/filter/request'
    js("URL.createObjectURL=window.analysisCreateURL;HTMLAnchorElement.prototype.click=window.analysisAnchorClick")
    # One scrolling surface and accessible controls at a narrow viewport.
    command('/window/rect', {'width': 520, 'height': 900})
    assert js("const page=document.querySelector('.analysis-page');return page.scrollWidth<=page.clientWidth+1 && document.querySelector('.analysis-scroll').scrollWidth<=document.querySelector('.analysis-scroll').clientWidth+1"), 'Analysis overflows at narrow width'
    Path('/tmp/proteus-analysis-mobile.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    command('/window/rect', {'width': 1440, 'height': 1000})
    # Restore the real API. This also exercises removal of a now-absent turn filter.
    js("window.fetch=window.analysisFetch;window.analysisFixture=null")
    js(f"const root={root};root.querySelector('.usage-filters button').click();root.querySelector('.usage-footer button').click()")
    wait_for(lambda: js(f"return {root}.querySelectorAll('.usage-request').length===2"), 'Report did not restore real journal data')
    Path('/tmp/proteus-session-analysis.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    print('PASS: session analysis tabs/reload; scoped totals; diagnostic filters/search/sort; stable request identity; exact JSON export; narrow layout', flush=True)


def check_selection(command, js, wait_for):
    """Two real persisted sessions: analysis selection must not resume the active chat."""
    import json
    original = js("return document.querySelector('#analysis-session').value")
    js("document.querySelector('.analysis-open-chat').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"), 'Original chat did not open')
    js("document.querySelector('[aria-label=\"Новая сессия\"]').click()")
    wait_for(lambda: js("return new URL(location.href).searchParams.get('session_dir')") not in [None, original], 'New session was not created')
    second = js("return new URL(location.href).searchParams.get('session_dir')")
    wait_for(lambda: js("return !document.querySelector('.composer-stop') && document.querySelector('.connection-badge').classList.contains('completed')"), 'New session not ready')
    js("const area=document.querySelector('.composer textarea');area.value='Вторая сессия для анализа';area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Second session draft not ready')
    js("document.querySelector('.composer-submit').click()")
    wait_for(lambda: js("return document.querySelector('.results-panel').textContent.includes('Абзац 31:') && !document.querySelector('.composer-stop')"), 'Second session did not finish')
    js("document.querySelector('.sidebar-footer a[href=\"/context\"]').click()")
    root = "document.querySelector('.usage-details-host > div')?.shadowRoot"
    wait_for(lambda: js(f"return {root}?.querySelectorAll('.usage-request').length===1"), 'Second session report not shown')
    wait_for(lambda: js("return [...document.querySelector('#analysis-session').options].some(item=>item.value===" + json.dumps(original) + ")"), 'Original session missing from selector')
    js("const select=document.querySelector('#analysis-session');select.value=" + json.dumps(original) + ";select.dispatchEvent(new Event('change',{bubbles:true}))")
    wait_for(lambda: js(f"return {root}?.querySelectorAll('.usage-request').length===2"), 'Selector retained the other session report')
    assert js("return new URL(location.href).searchParams.get('session_dir')===" + json.dumps(second)), 'Analysis selection resumed a different active chat'
    command('/refresh', {})
    wait_for(lambda: js(f"return {root}?.querySelectorAll('.usage-request').length===2"), 'Reload lost the independently selected analysis session')
    assert js("return document.querySelector('#analysis-session').value===" + json.dumps(original) + " && new URL(location.href).searchParams.get('session_dir')===" + json.dumps(second)), 'Reload conflated active and inspected session'
    # The report and session summaries load independently after a cold reload.
    wait_for(lambda: js("const button=document.querySelector('.analysis-open-chat');return button && !button.disabled"), 'Selected session summary did not become available')
    js("document.querySelector('.analysis-open-chat').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea') && new URL(location.href).searchParams.get('session_dir')===" + json.dumps(original)), 'Open dialogue did not resume the inspected session')
    js("document.querySelector('.sidebar-footer a[href=\"/context\"]').click()")
    wait_for(lambda: js(f"return {root}?.querySelectorAll('.usage-request').length===2"), 'Original analysis did not reopen')
    print('PASS: independent analysis of two real sessions; reload retains inspected session; open dialogue resumes the inspected chat', flush=True)
