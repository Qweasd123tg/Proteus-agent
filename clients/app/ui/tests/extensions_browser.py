#!/usr/bin/env python3
"""Real Firefox + built Leptos + isolated subscription app-server + loopback provider. No live account.

Requires Firefox and geckodriver (PATH or GECKODRIVER). Only stdlib Python.
Run after trunk build and cargo build -p proteus-core -p proteus-reference-module.
"""
import base64
from images_checks import run as check_images
from markdown_checks import run as check_markdown, FIXTURE as MARKDOWN_FIXTURE
from simplify_checks import run as check_simplify
from settings_checks import run as check_settings
from interface_settings_checks import run as check_interface_settings
from client_modules_checks import run as check_client_modules
from extensions_checks import run as check_extensions
from panel_checks import run as check_panels
from workspace_checks import run as check_workspace
from motion_checks import run as check_motion
from select_checks import run as check_selects
from layout_checks import run as check_layout
from session_checks import run as check_session, check_inspector_startup, BOOTSTRAP
from queue_checks import run as check_queue
from live_checks import run as check_live
from session_switch_checks import run as check_session_switch
from chrome_checks import run as check_chrome
from placement_checks import run as check_placement
from polish_checks import run as check_polish, check_restore_failure
from tool_chain_checks import run as check_tool_chain
from approval_checks import run as check_approvals
from subagent_tab_checks import run as check_subagent_tabs
from typing_checks import run as check_typing
from scroll_jitter_checks import run as check_scroll_jitter
from planning_checks import run as check_planning
from usage_checks import run as check_usage
from architecture_checks import run as check_architecture
from agent_settings_checks import run as check_agent_settings
from notifications_checks import run as check_notifications
from turn_issue_checks import run as check_turn_issue
from chat_search_checks import run as check_chat_search
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import select
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, build_opener, ProxyHandler

ROOT = Path(__file__).resolve().parents[4]
HTTP = build_opener(ProxyHandler({}))


def request(url, method="GET", body=None):
    data = None if body is None else json.dumps(body).encode()
    req = Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with HTTP.open(req, timeout=35) as response:
            return json.load(response)
    except HTTPError as error:
        raise AssertionError(error.read().decode()) from error


def wait_for(check, description):
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.1)
    raise AssertionError(description)


def stop(process):
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


class Assets(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        if self.path != '/responses':
            self.send_error(404); return
        assert self.headers.get('Authorization') == 'Bearer fixture-access'
        self.server.model_inputs.append(json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0)))))
        count = getattr(self.server, 'model_requests', 0)
        self.server.model_requests = count + 1
        if not self.server.model_gate.wait(timeout=60):
            raise AssertionError('Queue fixture held the model request too long')
        failure = getattr(self.server, 'model_failure', None)
        if failure is not None:
            status, body = failure
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps(body).encode())
            return
        if count == 0:
            output = [{"type":"function_call","call_id":"ui-plan","name":"update_plan","arguments":json.dumps({"plan":[{"step":"Проверить панели","status":"completed"},{"step":"Проверить настройки","status":"completed"}]})}]
        else:
            output = [{"id":"ui-answer","type":"message","role":"assistant","content":[{"type":"output_text","text":"Проверка интерфейса завершена.\n\n- Панели раскрываются одним изменением ширины.\n- Расширения настраиваются в отдельном разделе.\n- Поле ввода оставляет место для последних сообщений.\n\n```rust\nfn main() {\n    println!(\"Proteus UI fixture\");\n}\n```"}]}]
        if count == 1 and '--markdown-only' in sys.argv:
            output[0]['content'][0]['text'] += MARKDOWN_FIXTURE
        if '--approval-only' in sys.argv:
            calls={0:('write_file',{'path':'approval-result.txt','content':'approved once'}),2:('exec_command',{'cmd':'printf approval-command','max_output_tokens':100}),4:('write_file',{'path':'denied-result.txt','content':'must not exist'})}
            if count in calls:
                name,args=calls[count];output=[{'type':'function_call','call_id':f'approval-{count}','name':name,'arguments':json.dumps(args)}]
        if count >= 2 and '--approval-only' not in sys.argv and '--images-only' not in sys.argv:
            chunks = getattr(self.server, 'typing_chunks', None) or [f"Абзац {i}: " + "Продолжение ответа. " * 8 + "\n\n" for i in range(32)]
            output = [{"id":f"ui-answer-{count}","type":"message","role":"assistant","content":[{"type":"output_text","text":''.join(chunks)}]}]
        if '--images-only' in sys.argv:
            images = [part for item in self.server.model_inputs[-1].get('input', []) for part in item.get('content', []) if part.get('type') == 'input_image']
            expected = (ROOT / 'crates/proteus-core/tests/fixtures/pixel.png').read_bytes()
            assert all(base64.b64decode(part['image_url'].split(',', 1)[1]) == expected for part in images)
            output = [{"id":f"image-answer-{count}","type":"message","role":"assistant","content":[{"type":"output_text","text":f"Изображений: {len(images)}"}]}]
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()
        if count >= 2 and '--approval-only' not in sys.argv and '--images-only' not in sys.argv:
            def emit(name, data):
                data['type'] = name
                self.wfile.write(('event: '+name+'\ndata: '+json.dumps(data)+'\n\n').encode())
                self.wfile.flush()
            emit('response.output_item.added', {"output_index":0,"item":{"id":output[0]['id'],"type":"message","role":"assistant","content":[]}})
            for index, chunk in enumerate(chunks):
                typing_gate = getattr(self.server, 'typing_gate', None)
                if typing_gate is not None and not typing_gate.acquire(timeout=60):
                    raise AssertionError('Typing fixture held a text chunk too long')
                emit('response.output_text.delta', {"output_index":0,"item_id":output[0]['id'],"content_index":0,"delta":chunk})
                if index == 3 and not self.server.stream_gate.wait(timeout=60):
                    raise AssertionError('Streaming fixture held the response too long')
                time.sleep(.1)
            typing_completed = getattr(self.server, 'typing_completed', None)
            if typing_completed is not None and not typing_completed.wait(timeout=60):
                raise AssertionError('Typing fixture held completion too long')
        self.wfile.write(('event: response.completed\ndata: '+json.dumps({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":100,"output_tokens":40,"input_tokens_details":{"cached_tokens":60},"output_tokens_details":{"reasoning_tokens":10}}}})+'\n\n').encode())

    def do_GET(self):
        path = self.path.split('?', 1)[0]
        inspector = ROOT / 'clients/app/diagnostics/dist'
        if (path=='/' and 'embedded=true' in self.path) or path in ('/architecture', '/inspector.html') or (not (Path(self.directory) / path.lstrip('/')).exists() and (inspector / path.lstrip('/')).is_file()):
            original = self.directory
            self.directory = str(inspector)
            if path=='/' or path in ('/architecture', '/inspector.html'):
                self.path = '/index.html'
            try:
                super().do_GET()
            finally:
                self.directory = original
            return
        if self.path.split('?', 1)[0] in ['/context', '/sessions', '/settings']:
            self.path = '/index.html'
        if path in ('/window-chrome.js', '/window-chrome.css'):
            self.send_response(200)
            self.send_header('Content-Type', 'text/javascript' if path.endswith('.js') else 'text/css')
            self.end_headers()
            self.wfile.write((ROOT / 'clients/app/launcher' / path.lstrip('/')).read_bytes())
            return
        if path == '/pending-bootstrap.html':
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.end_headers()
            script = "<script>const fetchLive=window.fetch;window.fetch=(input,...args)=>new URL(input.url||input,location.href).pathname==='/bootstrap'?(window.bootstrapBlocked=true,new Promise(()=>{})):fetchLive(input,...args);</script>"
            html = (ROOT / 'clients/app/ui/dist/index.html').read_text().replace('<head>', '<head>'+script)
            self.wfile.write(html.encode())
            return
        if path in ('/foundation.html', '/inspector-foundation.html'):
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.end_headers()
            client = inspector if path == '/inspector-foundation.html' else ROOT / 'clients/app/ui/dist'
            html = (client / 'index.html').read_text().replace('<head>', '<head>'+BOOTSTRAP)
            self.wfile.write(html.encode())
            return
        if self.path.startswith('/models?') or self.path == '/wham/usage':
            assert self.headers.get('Authorization') == 'Bearer fixture-access'
            assert self.headers.get('ChatGPT-Account-Id') == 'fixture-account'
            if self.path.startswith('/models?'):
                data = {"models": [{"slug": "fixture-model", "display_name": "Fixture", "visibility": "list", "priority": 0, "supported_reasoning_levels": []}, {"slug":"fixture-model-2","display_name":"Fixture 2","visibility":"list","priority":1,"supported_reasoning_levels":[{"effort":"low","description":"Low"},{"effort":"high","description":"High"}]}]}
            else:
                data = {"plan_type": "plus", "rate_limit": {"allowed": True, "limit_reached": False,
                    "primary_window": {"used_percent": 27, "limit_window_seconds": 18000, "reset_at": int(time.time()) + 3600},
                    "secondary_window": {"used_percent": 63, "limit_window_seconds": 604800, "reset_at": int(time.time()) + 86400}},
                    "additional_rate_limits": [{"metered_feature": "review", "limit_name": "Code review", "rate_limit": {
                        "allowed": False, "limit_reached": True, "primary_window": {"used_percent": 105, "limit_window_seconds": 900, "reset_at": int(time.time()) - 1}}}],
                    "credits": {"has_credits": True, "unlimited": False, "balance": "12.50"}}
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps(data).encode())
        elif self.path.startswith('/fixture/client/'):
            self.send_response(200)
            if self.path.endswith('extension.json'):
                data = json.dumps({"apiVersion":1,"id":"client-test","name":"Своя диагностика","description":"Browser fixture","entry":"./page.js","requires":["client.composer","agent.config.read"],"surfaces":["settings","composer-model"],"navigation":{"group":"diagnostics","icon":"analysis"}})
                self.send_header('Content-Type','application/json')
            else:
                data = """export async function mount({root,surface,services,signal}) {
                  window.clientMounts=(window.clientMounts||0)+1;
                  signal.addEventListener('abort',()=>window.clientAborts=(window.clientAborts||0)+1);
                  const input=document.createElement('input');input.dataset.customModule=surface;input.value=services['client.composer'].read().model;root.append(input);
                  await services['agent.config.read'].read();root.dataset.configRead='true';
                  return()=>window.clientDisposals=(window.clientDisposals||0)+1;
                }"""
                self.send_header('Content-Type','text/javascript')
            self.end_headers();self.wfile.write(data.encode())
        elif self.path.startswith('/fixture/'):
            self.send_response(200)
            if self.path.endswith('extension.json'):
                slow = '/slow/' in self.path
                data = json.dumps({"apiVersion": 1, "id": 'slow-test' if slow else 'external-test', "name": 'Медленная панель' if slow else 'Внешняя панель', "description": "Browser fixture", "entry": "./panel.js", "requires": []})
                self.send_header('Content-Type', 'application/json')
            elif '/slow/' in self.path:
                data = '''export async function mount({root}) {
                  window.slowMounts = (window.slowMounts || 0) + 1;
                  const p = document.createElement('p'); p.textContent = 'current panel'; root.append(p);
                  if (window.slowMounts === 1) await new Promise(resolve => window.finishSlowMount = resolve);
                  return () => { root.replaceChildren(); window.slowDisposals = (window.slowDisposals || 0) + 1; };
                }'''
                self.send_header('Content-Type', 'text/javascript')
            else:
                data = '''export function mount({root, signal}) {
                  const p = document.createElement('p'); p.textContent = 'External package'; root.append(p);
                  window.externalMounted = (window.externalMounted || 0) + 1;
                  signal.addEventListener('abort', () => window.externalAborted = (window.externalAborted || 0) + 1);
                  return () => window.externalDisposed = (window.externalDisposed || 0) + 1;
                }'''
                self.send_header('Content-Type', 'text/javascript')
            self.end_headers()
            self.wfile.write(data.encode())
        elif self.path == '/standalone.html':
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.end_headers()
            self.wfile.write(b'''<div id="host"></div><script type="module">
              import {mountExtensions} from '/extensions/host.js';
              window.stopExtensions = mountExtensions(document.querySelector('#host'));
            </script>''')
        else:
            super().do_GET()


def main():
    driver_binary = os.environ.get('GECKODRIVER') or shutil.which('geckodriver')
    if not driver_binary:
        cached = list((Path.home() / '.cache/selenium/geckodriver/linux64').glob('*/geckodriver'))
        driver_binary = str(sorted(cached)[-1]) if cached else None
    assert driver_binary, 'Set GECKODRIVER or install geckodriver on PATH'
    with tempfile.TemporaryDirectory(prefix='proteus-ui-extensions-') as temporary:
        folder = Path(temporary)
        server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Assets, directory=str(os.environ.get('PROTEUS_UI_TEST_DIST',ROOT / 'clients/app/ui/dist'))))
        server.model_inputs = []
        server.model_gate = threading.Event()
        server.model_gate.set()
        server.stream_gate = threading.Event()
        server.stream_gate.set()
        threading.Thread(target=server.serve_forever, daemon=True).start()
        web = f'http://127.0.0.1:{server.server_port}'
        auth = folder / 'fixture-auth.json'
        auth.write_text(json.dumps({"access_token": "fixture-access", "refresh_token": "fixture-refresh", "account_id": "fixture-account", "expires_at": 9000000000}))
        config = folder / 'subscription.toml'
        config.write_text('''active_provider = "subscription"
[profile]
name = "extensions-smoke"
[providers.subscription]
provider = "custom-model"
model = "fixture-model"
[components.model]
command = "proteus-reference-module"
[components.model.exports.model.custom-model]
[components.model.exports.workflow."coding.single_loop"]
[components.model.exports.tool."reference.tools"]
[components.model.exports.policy.allow_all]
[modules]
workflow = "coding.single_loop"
policy = "allow_all"
[tools]
enabled = ["update_plan"]
[[tools.configured]]
name = "fixture_managed"
description = "Runtime-managed tool absent from tools.enabled"
safety = "ReadOnly"
[tools.configured.executor]
kind = "process"
command = "/bin/true"
[module_config.model.custom-model]
implementation = "openai_codex"
base_url = ''' + json.dumps(web) + '\nquota_url = ' + json.dumps(web + '/wham/usage') + '\nauth_file = ' + json.dumps(str(auth)) + '\n[event_log]\npath = ' + json.dumps(str(folder / 'events.jsonl')) + '\n')
        if '--images-only' in sys.argv:
            config.write_text(config.read_text()+'\n[module_config.model.custom-model.capabilities]\nsupports_image_input = true\n')
        if '--approval-only' in sys.argv:
            config.write_text(config.read_text().replace('policy.allow_all','policy.ask_write').replace('policy = "allow_all"','policy = "ask_write"').replace('enabled = ["update_plan"]','enabled = ["update_plan", "write_file", "exec_command"]'))
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            driver_port = sock.getsockname()[1]
        endpoint = f'http://127.0.0.1:{driver_port}'
        backend = driver = None
        session = None
        with (folder / 'backend.log').open('w+') as backend_log, (folder / 'browser.log').open('w+') as browser_log:
            try:
                env = os.environ.copy()
                env.pop('PROTEUS_CONFIG_PATH', None)
                env.update(PATH=str(ROOT / 'target/debug') + ':' + env['PATH'], PROTEUS_CONFIG_HOME=str(folder / 'config'), XDG_CONFIG_HOME=str(folder / 'settings'), XDG_DATA_HOME=str(folder / 'data'))
                (folder / 'preview-fixture').mkdir()
                tracked = folder / 'preview-fixture' / 'hello world.txt'
                tracked.write_text('<b>Старое</b>\nФайл только для чтения\n')
                deleted = folder / 'preview-fixture' / 'deleted.txt'
                deleted.write_text('Удалённая строка\n')
                for args in [['init', '--quiet'], ['add', 'preview-fixture'], ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--quiet', '-m', 'Fixture baseline']]:
                    subprocess.run(['git', '-C', str(folder), *args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                tracked.write_text('<b>Привет</b>\nФайл только для чтения\n')
                deleted.unlink()
                (folder / 'preview-fixture' / 'delayed.txt').write_text('Delayed file fixture')
                backend = subprocess.Popen([str(ROOT / 'target/debug/proteus'), '--config', str(config), '--cwd', str(folder), 'server', 'http', '--port', '0', '--token', 'extension-smoke', '--ready-stdout', '--allow-origin', web], env=env, stdout=subprocess.PIPE, stderr=backend_log, text=True, start_new_session=True)
                origin = None
                deadline = time.monotonic() + 40
                while time.monotonic() < deadline and backend.poll() is None:
                    if select.select([backend.stdout], [], [], 0.1)[0]:
                        line = backend.stdout.readline()
                        if line.startswith('{'):
                            record = json.loads(line)
                            if record.get('type') == 'http_ready':
                                origin = record['origin']
                                break
                assert origin, 'Backend did not become ready'
                driver = subprocess.Popen([driver_binary, '--port', str(driver_port)], stdout=browser_log, stderr=browser_log, start_new_session=True)
                def driver_ready():
                    try:
                        return request(endpoint + '/status')['value']['ready']
                    except OSError:
                        return False
                wait_for(driver_ready, 'geckodriver startup')
                reduced_motion = 1 if '--reduced-motion' in sys.argv else int(os.environ.get('PROTEUS_TEST_REDUCED_MOTION', '0'))
                capabilities = {'capabilities': {'alwaysMatch': {'browserName': 'firefox', 'pageLoadStrategy': 'eager', 'moz:firefoxOptions': {'args': ['-headless'], 'prefs': {'network.proxy.type': 0, 'ui.prefersReducedMotion': reduced_motion}}}}}
                session = request(endpoint + '/session', 'POST', capabilities)['value']['sessionId']
                url = endpoint + '/session/' + session
                def command(path, body):
                    return request(url + path, 'GET' if body is None else 'POST', body)['value']
                def js(script):
                    return command('/execute/sync', {'script': script, 'args': []})
                def loaded():
                    return js("return document.querySelector('[data-extension-id=agent-info] .extension-panel-content')?.shadowRoot?.textContent.includes('extensions-smoke')")
                command('/window/rect', {'width': 1440, 'height': 1000})
                assert js("return matchMedia('(prefers-reduced-motion: reduce)').matches") == bool(reduced_motion), 'Browser did not apply motion preference'
                if '--images-only' in sys.argv:
                    command('/url', {'url':web+'/?'+urlencode({'server':origin,'token':'extension-smoke'})})
                    wait_for(loaded, 'Client missing')
                    check_images(command, js, wait_for, server, web, origin)
                    return
                if '--approval-only' in sys.argv:
                    command('/url', {'url':web+'/?'+urlencode({'server':origin,'token':'extension-smoke'})})
                    wait_for(loaded,'Client missing')
                    check_approvals(command,js,wait_for,folder)
                    return
                if '--agent-settings-only' in sys.argv:
                    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                    wait_for(loaded, 'Client missing')
                    def capture(name):
                        time.sleep(0.5)  # let the page transition settle
                        Path(f'/tmp/proteus-agent-settings-{name}.png').write_bytes(base64.b64decode(request(url + '/screenshot')['value']))
                    check_agent_settings(command, js, wait_for, config, capture)
                    return
                if '--preference-error-only' in sys.argv:
                    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                    wait_for(loaded,'Client missing')
                    js("localStorage.setItem('proteus.model.last-selection',JSON.stringify({model:'fixture-model',effort:'none'}))")
                    check_restore_failure(command,js,wait_for)
                    return
                if '--chrome-only' in sys.argv:
                    check_chrome(command, js, wait_for, web, origin)
                    return
                if '--motion-only' in sys.argv:
                    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                    wait_for(loaded, 'Client did not load for motion checks')
                    check_motion(command, js, wait_for)
                    return
                if '--workspace-layout-only' in sys.argv:
                    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                    wait_for(loaded, 'Client did not load for workspace checks')
                    check_workspace(command, js, wait_for)
                    return
                if '--workspace-only' in sys.argv:
                    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                    wait_for(loaded, 'Client did not load for workspace checks')
                    check_panels(command, js, wait_for)
                    return
                if '--simplify-only' in sys.argv:
                    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                    wait_for(loaded, 'Client did not load')
                    check_simplify(command, js, wait_for, web, origin)
                    return
                if '--inspector-only' in sys.argv:
                    check_architecture(command, js, wait_for, web, origin)
                    return
                if '--subagents-only' in sys.argv:
                    check_subagent_tabs(command, js, wait_for, web, origin)
                    return
                if '--typing-only' in sys.argv:
                    check_typing(command, js, wait_for, web, origin, server)
                    return
                if '--scroll-jitter-only' in sys.argv:
                    check_scroll_jitter(command, js, wait_for, web, origin)
                    return
                if '--modules-only' in sys.argv:
                    check_client_modules(command,js,wait_for,web,origin,loaded)
                    return
                check_extensions(command, js, wait_for, web, origin, loaded)
                if '--placement-only' in sys.argv:
                    check_placement(command,js,wait_for)
                    check_settings(command,js,wait_for)
                    check_chrome(command,js,wait_for,web,origin)
                    return
                if '--markdown-only' in sys.argv:
                    check_markdown(command,js,wait_for)
                    return
                if '--choices-only' in sys.argv:
                    check_selects(command,js,wait_for)
                    check_tool_chain(command,js,wait_for)
                    return
                if '--tool-chain-only' in sys.argv:
                    check_tool_chain(command, js, wait_for)
                    return
                if '--polish-only' in sys.argv:
                    check_tool_chain(command, js, wait_for)
                    check_polish(command, js, wait_for)
                    check_settings(command, js, wait_for)
                    check_chrome(command, js, wait_for, web, origin)
                    return
                if '--desktop-layout-only' in sys.argv:
                    check_settings(command, js, wait_for)
                    check_panels(command, js, wait_for)
                    check_layout(command, js, wait_for)
                    check_architecture(command, js, wait_for, web, origin)
                    return
                if '--preferences-only' in sys.argv:
                    check_interface_settings(command, js, wait_for, server)
                    check_settings(command, js, wait_for)
                    check_chrome(command, js, wait_for, web, origin)
                    return
                if '--settings-only' in sys.argv:
                    check_settings(command, js, wait_for)
                    return
                if '--usage-shell-only' in sys.argv:
                    check_usage(command, js, wait_for)
                    check_layout(command, js, wait_for)
                    return
                if '--sessions-only' in sys.argv:
                    check_session(command, js, wait_for, web, origin, loaded)
                    check_inspector_startup(command, js, wait_for, web, origin)
                    return
                if '--stability-only' in sys.argv:
                    check_session(command, js, wait_for, web, origin, loaded)
                    check_live(command, js, wait_for, server)
                    check_session_switch(command, js, wait_for, server, origin)
                    print('PASS: client stability, long history, streaming, reconnect and timer lifecycle', flush=True)
                    return
                if '--session-switch-only' in sys.argv:
                    check_session_switch(command, js, wait_for, server, origin)
                    return
                if '--notifications-only' in sys.argv:
                    check_notifications(command, js, wait_for, server)
                    return
                if '--chat-search-only' in sys.argv:
                    def capture(name):
                        time.sleep(0.3)
                        Path(f'/tmp/proteus-{name}.png').write_bytes(base64.b64decode(request(url + '/screenshot')['value']))
                    check_chat_search(command, js, wait_for, server, capture)
                    return
                if '--turn-issue-only' in sys.argv:
                    def capture(name):
                        time.sleep(0.3)
                        Path(f'/tmp/proteus-{name}.png').write_bytes(base64.b64decode(request(url + '/screenshot')['value']))
                    check_turn_issue(command, js, wait_for, server, capture)
                    return
                check_selects(command, js, wait_for)
                check_panels(command, js, wait_for)
                if '--shell-only' in sys.argv:
                    check_layout(command, js, wait_for)
                    print('PASS: shell navigation, widget placement, menus, focus, scrolling and responsive layout', flush=True)
                    return
                check_usage(command, js, wait_for)
                check_layout(command, js, wait_for)
                screenshot = request(url + '/screenshot')['value']
                Path('/tmp/proteus-ui-extensions.png').write_bytes(base64.b64decode(screenshot))
                if '--panels-only' in sys.argv:
                    print('PASS: extensions, custom selectors, owned file panels, motion and layout', flush=True)
                    return
                check_architecture(command, js, wait_for, web, origin)
                check_session(command, js, wait_for, web, origin, loaded)
                check_queue(command, js, wait_for, server)
                check_live(command, js, wait_for, server)
                check_session_switch(command, js, wait_for, server, origin)
                check_planning(command, js, wait_for, server, request, origin, ROOT, config, folder, env, stop)
                stop(backend)
                js("document.querySelector('[data-extension-id=model-quota] .extension-panel-content').shadowRoot.querySelector('button').click()")
                wait_for(lambda: js("const root=document.querySelector('[data-extension-id=model-quota] .extension-panel-content').shadowRoot; return root.textContent.includes('Не удалось получить лимиты') && root.querySelectorAll('progress').length === 0"), 'Quota error retained old balances')
                print('PASS: settings/panel separation; extension install/lifecycle/persistence; quota API; panel layout and resize; settings save/rollback; independent host')
            except Exception:
                if session:
                    Path('/tmp/proteus-ui-failure.png').write_bytes(base64.b64decode(request(url + '/screenshot')['value']))
                for log in [backend_log, browser_log]:
                    log.flush(); log.seek(0); print(log.read()[-5000:])
                raise
            finally:
                if session:
                    try:
                        request(endpoint + '/session/' + session, 'DELETE')
                    except Exception:
                        pass
                stop(driver)
                stop(backend)
                server.shutdown()
                server.server_close()


if __name__ == '__main__':
    main()
