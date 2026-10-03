"""Approval scope picker keeps canonical responses and actual tool effects."""
import base64
from pathlib import Path


def run(command, js, wait_for, folder):
    js("window.approvalReplies=[];const realFetch=window.fetch;window.fetch=async(input,init)=>{if(new URL(input.url||input,location.href).pathname==='/approval')approvalReplies.push(init?.body?JSON.parse(init.body):await input.clone().json());return realFetch(input,init)}")

    def send(text):
        js("const t=document.querySelector('.composer textarea');t.value="+repr(text)+";t.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return document.querySelector('.composer-submit')?.disabled===false"), 'Submit not ready')
        js("document.querySelector('.composer-submit').click()")
        try:
            wait_for(lambda: js("return !!document.querySelector('.approval-card select')"), 'Approval missing')
        except AssertionError as error:
            raise AssertionError(js("return document.body.innerText")) from error

    def choose(value):
        # Click through the production themed picker so input/change reaches Leptos.
        js("document.querySelector('.approval-card select').click()")
        wait_for(lambda: js("return !!document.querySelector('.select-picker')?.matches(':popover-open')"), 'Approval picker missing')
        index = js("return [...document.querySelector('.approval-card select').options].findIndex(o=>o.value==="+repr(value)+")")
        js("document.querySelectorAll('.select-picker [role=option]')["+str(index)+"].click()")
        wait_for(lambda: js("return document.querySelector('.approval-card select').value==="+repr(value)), 'Approval scope did not update')

    def settle():
        wait_for(lambda: js("return !document.querySelector('.approval-card') && !document.querySelector('.composer-stop')"), 'Approval did not settle')

    send('Запиши тестовый файл')
    assert js("return document.querySelector('.approval-card select').value==='none' && document.querySelector('.approval-card .approval-cwd').textContent && document.querySelector('.approval-preview').textContent.includes('approved once') && !document.querySelector('.approval-arguments').open"), 'Approval hides action or silently grants repeat access'
    choose('workspace_write')
    assert js("return document.querySelector('.approval-scope p').textContent.includes('инструмента')"), 'Broad scope explanation absent'
    choose('exact')
    js("document.querySelector('.approval-arguments summary').click()")
    assert js("return document.querySelector('.approval-arguments').open && document.querySelector('.approval-arguments').textContent.includes('approved once')"), 'Raw parameters unavailable'
    Path('/tmp/proteus-approval-refined.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("document.querySelector('.approval-card .btn-primary').click()")
    settle()
    assert js("return approvalReplies.at(-1).approved && approvalReplies.at(-1).cache==='exact_call'"), 'Write exact scope changed on the wire'
    assert (folder/'approval-result.txt').read_text() == 'approved once'

    send('Выполни тестовую команду')
    assert js("return document.querySelector('.approval-card select').options.length===2 && document.querySelector('.approval-card select').value==='none' && !document.querySelector('.approval-preview') && document.querySelector('.approval-card .tool-preview').textContent.includes('printf approval-command')"), 'Command inherited previous approval scope, broad option or lost preview'
    choose('exact')
    js("document.querySelector('.approval-card .btn-primary').click()")
    settle()
    assert js("return approvalReplies.at(-1).approved && approvalReplies.at(-1).cache==='exact_command'"), 'Command scope changed on the wire'

    send('Отклони тестовую запись')
    choose('workspace_write')
    js("document.querySelector('.approval-card .danger').click()")
    settle()
    assert js("return !approvalReplies.at(-1).approved && approvalReplies.at(-1).cache==='none'"), 'Denial retained a broad permission'
    assert not (folder/'denied-result.txt').exists(), 'Denied write executed'
    js("const chain=[...document.querySelectorAll('.tool-chain')].at(-1);if(!chain.classList.contains('expanded'))chain.querySelector('.tool-chain-toggle').click()")
    wait_for(lambda: js("return !!document.querySelector('.tool-card-summary .status-badge.failed')"), 'Denied call lacks visible status')
    reason = js("return [...document.querySelectorAll('.tool-card-summary')].find(x=>x.querySelector('.status-badge.failed'))?.querySelector('.tool-card-reason')?.textContent||''")
    print('Denied reason:', reason, flush=True)
    assert reason.strip(), 'Denied call hides its reason until expanded'
    print('PASS: actual write and command approvals; themed picker -> exact_call/exact_command; reset per request; denial always none with no file effect; preview and raw parameters; visible denial with its reason', flush=True)
