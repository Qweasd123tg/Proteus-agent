"""Control and admission ordering in the built client with real HTTP/SSE."""
import json


def run(command, js, wait_for, server):
    def send(text):
        js("const input=document.querySelector('.composer-input textarea');input.value=" + json.dumps(text) + ";input.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Draft did not enable submit')
        js("document.querySelector('.composer-submit').click()")

    wait_for(lambda: js("return !!document.querySelector('.composer-model-menu .menu-option-row')"), 'Model choices missing')
    js("window.raceFetch=window.fetch;window.modelPosts=0;window.modelLog=[];window.releaseModel=null;window.fetch=async(input,...args)=>{if(new URL(input.url||input,location.href).pathname!='/model')return raceFetch(input,...args);window.modelPosts++;modelLog.push(await input.clone().text());const response=await raceFetch(input,...args);if(window.modelPosts===1)return new Promise(resolve=>window.releaseModel=()=>resolve(response));return response};document.querySelector('.composer-model-menu summary').click();[...document.querySelectorAll('.composer-model-menu .menu-option-row')].find(b=>b.textContent.includes('Fixture 2')).click()")
    wait_for(lambda: js("return typeof releaseModel==='function'"), 'First model response was not held')
    js("[...document.querySelectorAll('.composer-model-menu .menu-option-row')].find(b=>b.querySelector('.menu-option-title').textContent==='Fixture').click()")
    observed=js("return {posts:window.modelPosts,requests:modelLog,selected:document.querySelector('.composer-model-menu summary').dataset.uiTooltip}")
    assert observed['posts']==1, f'Later model write overtook the pending write: {observed}'
    js("releaseModel()")
    wait_for(lambda: js("return window.modelPosts===2 && JSON.parse(localStorage.getItem('proteus.model.last-selection')||'null')?.model==='fixture-model'"), 'Newest model choice did not settle')
    js("window.fetch=raceFetch")

    # A definitive pre-admission failure must release the optimistic run.
    before = server.model_requests
    js("window.raceFetch=window.fetch;window.fetch=(input,...args)=>{if(new URL(input.url||input,location.href).pathname!='/send-async')return raceFetch(input,...args);window.fetch=raceFetch;return Promise.resolve(new Response('fixture rejected before admission',{status:404}))}")
    send('Rejected before admission')
    wait_for(lambda: js("return !document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes('fixture rejected before admission')"), 'Rejected send kept a phantom active run')
    assert server.model_requests == before, 'Rejected request reached the provider'

    # Capture the queued request before transport admission. Let the original
    # turn finish, then admit a new turn and withhold its HTTP acceptance until
    # its authoritative SSE settlement has already arrived.
    server.model_gate.clear()
    try:
        send('Original turn')
        wait_for(lambda: server.model_requests > before, 'Original turn did not reach the provider')
        js("window.raceFetch=window.fetch;window.queueDispatch=null;window.queueReply=null;window.fetch=(input,...args)=>{if(new URL(input.url||input,location.href).pathname!='/send-async')return raceFetch(input,...args);return new Promise(resolve=>window.queueDispatch=async()=>{const response=await raceFetch(input,...args);window.queueImmediate=(await response.clone().json()).output.queued===false;window.queueReply=()=>resolve(response)})}")
        send('Late admission')
        wait_for(lambda: js("return typeof queueDispatch==='function'"), 'Queue request was not deferred')
        server.model_gate.set()
        wait_for(lambda: js("return !document.querySelector('.composer-stop')"), 'Original turn did not settle')
        js("queueDispatch()")
        wait_for(lambda: js("return typeof queueReply==='function' && !document.querySelector('.composer-stop') && [...document.querySelectorAll('.user-message')].some(m=>m.textContent.includes('Late admission'))"), 'New turn did not settle before HTTP acceptance')
        js("window.queueReplySettled=false;window.admissionUsers=[...document.querySelectorAll('.user-message')].filter(m=>m.textContent.includes('Late admission')).length;queueReply();requestAnimationFrame(()=>requestAnimationFrame(()=>window.queueReplySettled=true));window.fetch=raceFetch")
        wait_for(lambda: js("return window.queueReplySettled && !document.querySelector('.composer-stop')"), 'Late acceptance resurrected the settled run')
        assert js("return window.queueImmediate"), 'Fixture did not exercise immediate admission of the queued request'
        assert js("return [...document.querySelectorAll('.user-message')].filter(m=>m.textContent.includes('Late admission')).length===admissionUsers"), 'Late acceptance duplicated the submitted message'
    finally:
        server.model_gate.set()
        js("if(window.raceFetch)window.fetch=raceFetch")

    # A completed GET can still be waiting in transport when deletion commits.
    wait_for(lambda: js("return !!document.querySelector('.session-item-shell')"), 'Saved session missing from the sidebar')
    js("window.deletedCatalogSession=document.querySelector('.session-item-shell').dataset.sessionDir;window.raceFetch=window.fetch;window.holdCatalog=true;window.releaseCatalog=null;window.fetch=async(input,...args)=>{const response=await raceFetch(input,...args);if(new URL(input.url||input,location.href).pathname!='/sessions'||!holdCatalog)return response;holdCatalog=false;window.staleCatalog=await response.clone().json();return new Promise(resolve=>window.releaseCatalog=()=>resolve(response))};document.querySelector('[aria-label=\"Обновить сессии\"]').click()")
    try:
        wait_for(lambda: js("return typeof releaseCatalog==='function'"), 'Catalog response was not held')
        assert js("return staleCatalog.some(s=>s.session_dir===deletedCatalogSession)"), 'Held catalog did not contain the session to delete'
        js("window.savedConfirm=window.confirm;window.confirm=()=>true;[...document.querySelectorAll('.session-item-shell')].find(row=>row.dataset.sessionDir===deletedCatalogSession).querySelector('[data-delete-session]').click();window.confirm=savedConfirm")
        wait_for(lambda: js("return ![...document.querySelectorAll('.session-item-shell')].some(row=>row.dataset.sessionDir===deletedCatalogSession)"), 'Session deletion did not reach the sidebar')
        js("window.catalogReplySettled=false;releaseCatalog();requestAnimationFrame(()=>requestAnimationFrame(()=>window.catalogReplySettled=true))")
        wait_for(lambda: js("return catalogReplySettled"), 'Held catalog did not settle')
        assert js("return ![...document.querySelectorAll('.session-item-shell')].some(row=>row.dataset.sessionDir===deletedCatalogSession)"), 'Late catalog restored the deleted session'
    finally:
        js("window.fetch=raceFetch")
    print('PASS: serialized model writes and newest selection; definite HTTP rejection; queued admission reply after real SSE settlement; stale catalog after deletion', flush=True)
