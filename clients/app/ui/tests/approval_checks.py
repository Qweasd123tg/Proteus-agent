"""Approval buttons keep canonical responses and actual tool effects."""
import base64
from pathlib import Path


def run(command, js, wait_for, folder):
    js("window.approvalReplies=[];const realFetch=window.fetch;window.fetch=async(input,init)=>{if(new URL(input.url||input,location.href).pathname==='/approval')approvalReplies.push(init?.body?JSON.parse(init.body):await input.clone().json());return realFetch(input,init)}")

    def send(text):
        js("const t=document.querySelector('.composer textarea');t.value="+repr(text)+";t.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return document.querySelector('.composer-submit')?.disabled===false"), 'Submit not ready')
        js("document.querySelector('.composer-submit').click()")
        try:
            wait_for(lambda: js("return !!document.querySelector('.approval-card [data-approval-scope]')"), 'Approval missing')
        except AssertionError as error:
            raise AssertionError(js("return document.body.innerText")) from error

    def scopes():
        return js("return [...document.querySelectorAll('.approval-card [data-approval-scope]')].map(b=>b.dataset.approvalScope)")

    def settle():
        wait_for(lambda: js("return !document.querySelector('.approval-card') && !document.querySelector('.composer-stop')"), 'Approval did not settle')

    send('Запиши тестовый файл')
    assert js("return !document.querySelector('.approval-card select') && document.querySelector('.approval-card .approval-cwd').textContent && document.querySelector('.approval-subject').textContent.includes('approval-result.txt') && document.querySelector('.approval-preview').textContent.includes('approved once') && !document.querySelector('.approval-arguments').open"), 'Approval hides the action or preselects repeat access'
    assert scopes() == ['exact', 'workspace_write', 'none'], scopes()
    assert js("return document.querySelector('[data-approval-scope=workspace_write]').dataset.uiTooltip.includes('инструмента')"), 'Broad scope explanation absent'
    js("document.querySelector('.approval-arguments summary').click()")
    assert js("return document.querySelector('.approval-arguments').open && document.querySelector('.approval-arguments').textContent.includes('approved once')"), 'Raw parameters unavailable'
    Path('/tmp/proteus-approval-refined.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("document.querySelector('[data-approval-scope=exact]').click()")
    settle()
    assert js("return approvalReplies.at(-1).approved && approvalReplies.at(-1).cache==='exact_call'"), 'Write exact scope changed on the wire'
    assert (folder/'approval-result.txt').read_text() == 'approved once'

    send('Выполни тестовую команду')
    assert scopes() == ['exact', 'none'], scopes()
    assert js("return !document.querySelector('.approval-preview') && document.querySelector('.approval-subject').textContent==='printf approval-command' && document.querySelector('[data-approval-scope=exact]').textContent.includes('команду')"), 'Command lost its headline or offered a broad option'
    js("document.querySelector('[data-approval-scope=exact]').click()")
    settle()
    assert js("return approvalReplies.at(-1).approved && approvalReplies.at(-1).cache==='exact_command'"), 'Command scope changed on the wire'

    send('Отклони тестовую запись')
    js("document.querySelector('.approval-card [data-approval-decision=deny]').click()")
    settle()
    assert js("return !approvalReplies.at(-1).approved && approvalReplies.at(-1).cache==='none'"), 'Denial retained a broad permission'
    assert not (folder/'denied-result.txt').exists(), 'Denied write executed'
    js("const chain=[...document.querySelectorAll('.tool-chain')].at(-1);if(!chain.classList.contains('expanded'))chain.querySelector('.tool-chain-toggle').click()")
    wait_for(lambda: js("return !!document.querySelector('.tool-card-summary .status-badge.failed')"), 'Denied call lacks visible status')
    reason = js("return [...document.querySelectorAll('.tool-card-summary')].find(x=>x.querySelector('.status-badge.failed'))?.querySelector('.tool-card-reason')?.textContent||''")
    print('Denied reason:', reason, flush=True)
    assert reason.strip(), 'Denied call hides its reason until expanded'
    print('PASS: actual write and command approvals; explicit repeat buttons -> exact_call/exact_command; no preselected scope; denial always none with no file effect; action headline, preview and raw parameters; visible denial with its reason', flush=True)
