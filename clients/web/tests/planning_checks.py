"""Explicit plan commands through Firefox, HTTP and stdio; cold journal replay."""
import json
from pathlib import Path
import queue
import subprocess
import threading
from urllib.parse import urlencode


def run(command, js, wait_for, server, request, origin, root, config, folder, env, stop):
    def draft(text):
        js("const a=document.querySelector('.composer-input textarea');a.value=" + json.dumps(text) + ";a.dispatchEvent(new Event('input',{bubbles:true}))")

    def complete():
        wait_for(lambda: js("return !document.querySelector('.composer-stop') && !!document.querySelector('.plan-actions-card')"), 'Plan did not settle with actions')

    js("document.querySelector('.composer-access-menu summary').click();[...document.querySelectorAll('.composer-access-menu button')].find(b=>b.textContent.includes('Планирование')).click()")
    wait_for(lambda: js("return document.querySelector('.composer-access-menu summary').textContent.includes('Планирование')"), 'Planning mode not selected')
    # Changing the selector is an explicit setting command. The following sends
    # must carry their own options and never perform a preparatory /mode call.
    js("window.planRequests=[];window.planFetch=window.fetch;window.fetch=async(input,init)=>{const p=String(input.url||input).split('?')[0];if(p.endsWith('/mode')||p.endsWith('/send-async'))window.planRequests.push({path:p,body:init?.body ? JSON.parse(init.body) : await input.clone().json()});return window.planFetch(input,init)}")
    topic = 'Спланируй перенос настроек без изменения файлов'
    before = len(server.model_inputs)
    draft(topic)
    js("document.querySelector('.composer-submit').click()")
    complete()
    sent = js('return window.planRequests')
    assert len(sent) == 1 and sent[0]['path'].endswith('/send-async'), sent
    assert sent[0]['body']['text'] == topic, sent
    assert sent[0]['body']['options'] == {'intent': 'planning.start', 'permission_mode': 'plan'}, sent
    session_dir = sent[0]['body']['session_dir']
    web_input = server.model_inputs[before]
    assert 'planning interview' in json.dumps(web_input)
    command('/refresh', {})
    complete()
    feedback = 'Добавь проверку второго окна'
    draft(feedback)
    js("document.querySelector('.plan-actions-card .secondary').click()")
    complete()
    js("document.querySelector('.plan-actions-card .btn-primary').click()")
    wait_for(lambda: js("return !document.querySelector('.composer-stop') && !document.querySelector('.plan-actions-card')"), 'Execute did not settle')
    print('PASS: raw planning text; atomic send options; plan actions restored after reload; revise and execute', flush=True)

    def api(path, body=None):
        return request(origin + path + '?' + urlencode({'token': 'extension-smoke', 'session_dir': session_dir}), 'GET' if body is None else 'POST', body)

    mode = api('/config')['permission_mode']
    before = len(server.model_inputs)
    result = api('/send', {'id': 'http-plan', 'text': topic, 'session_dir': session_dir, 'options': {'intent': 'planning.start', 'permission_mode': 'plan'}})
    assert result['ok'], result
    http_input = server.model_inputs[before]
    assert api('/config')['permission_mode'] == mode, 'Run changed session defaults'
    # Same named intention arrives through a real standalone stdio transport.
    with (folder / 'planning-stdio.log').open('w+') as log:
        child = subprocess.Popen([str(root / 'target/debug/proteus'), '--config', str(config), '--cwd', str(folder), '--new-session', 'server', 'stdio'], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, start_new_session=True)
        outputs = queue.Queue()
        def read():
            for line in child.stdout:
                outputs.put(json.loads(line))
        threading.Thread(target=read, daemon=True).start()
        def rpc(payload):
            child.stdin.write(json.dumps(payload) + '\n'); child.stdin.flush()
            while True:
                item = outputs.get(timeout=40)
                if item.get('type') == 'response' and item.get('id') == payload['id']:
                    assert item['ok'], item
                    return item.get('output')
        try:
            before = len(server.model_inputs)
            rpc({'type': 'send', 'id': 'stdio-plan', 'text': topic, 'options': {'intent': 'planning.start', 'permission_mode': 'plan'}})
            stdio_input = server.model_inputs[before]
            stdio_config = rpc({'type': 'config_summary', 'id': 'stdio-config'})
            stdio_dir = stdio_config['session_dir']
            assert stdio_config['permission_mode'] == 'Normal', stdio_config
            rpc({'type': 'shutdown', 'id': 'shutdown'})
            child.wait(timeout=10)
        finally:
            stop(child)
    # Compare only deterministic invocation facts, not live model prose or ids.
    for candidate in [http_input, stdio_input]:
        assert candidate['instructions'] == web_input['instructions'], (candidate, web_input)
        assert topic in json.dumps(candidate['input'], ensure_ascii=False)
    print('PASS: web, HTTP and stdio share planning instructions and preserve session defaults', flush=True)

    # Reject an unsupported action and retain an Error turn without a model call.
    before = len(server.model_inputs)
    failure = api('/send', {'id': 'bad-intent', 'text': 'Unsupported action', 'session_dir': session_dir, 'options': {'intent': 'unknown.action', 'permission_mode': 'normal'}})
    assert not failure['ok'] and 'unsupported workflow intent' in failure['error'], failure
    assert len(server.model_inputs) == before
    for source in [stdio_dir, session_dir]:
        records = [json.loads(line) for line in (Path(source) / 'journal.jsonl').read_text().splitlines()]
        opened = [r for r in records if r['kind'] == 'turn_opened'][-1]
        replay = subprocess.run([str(root / 'target/debug/proteus'), '--config', str(config), 'replay', 'workflow', source, '--turn-id', opened['turn_id'], '--json'], env=env, text=True, capture_output=True, timeout=40)
        assert replay.returncode == 0, replay.stderr
        report = json.loads(replay.stdout)
        assert report['comparison']['matched'], report
    assert len(server.model_inputs) == before, 'Replay contacted the model'
    print('PASS: Success/Error cold workflow replay retains intention without new model requests', flush=True)
