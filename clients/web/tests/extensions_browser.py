#!/usr/bin/env python3
"""Real Firefox + built Leptos + isolated subscription app-server + loopback provider. No live account.

Requires Firefox and geckodriver (PATH or GECKODRIVER). Only stdlib Python.
Run after trunk build and cargo build -p proteus-core -p proteus-reference-worker.
"""
import base64
from extensions_checks import run as check_extensions
from panel_checks import run as check_panels
from select_checks import run as check_selects
from layout_checks import run as check_layout
from session_checks import run as check_session, BOOTSTRAP
from queue_checks import run as check_queue
from live_checks import run as check_live
from planning_checks import run as check_planning
from usage_checks import run as check_usage
from architecture_checks import run as check_architecture
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

ROOT = Path(__file__).resolve().parents[3]
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
        if count == 0:
            output = [{"type":"function_call","call_id":"ui-plan","name":"update_plan","arguments":json.dumps({"plan":[{"step":"Проверить панели","status":"completed"},{"step":"Проверить настройки","status":"completed"}]})}]
        else:
            output = [{"id":"ui-answer","type":"message","role":"assistant","content":[{"type":"output_text","text":"Проверка интерфейса завершена.\n\n- Панели раскрываются одним изменением ширины.\n- Расширения настраиваются в отдельном разделе.\n- Поле ввода оставляет место для последних сообщений.\n\n```rust\nfn main() {\n    println!(\"Proteus UI fixture\");\n}\n```"}]}]
        if count >= 2:
            chunks = [f"Абзац {i}: " + "Продолжение ответа. " * 8 + "\n\n" for i in range(32)]
            output = [{"id":f"ui-answer-{count}","type":"message","role":"assistant","content":[{"type":"output_text","text":''.join(chunks)}]}]
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()
        if count >= 2:
            def emit(name, data):
                self.wfile.write(('event: '+name+'\ndata: '+json.dumps(data)+'\n\n').encode())
                self.wfile.flush()
            emit('response.output_item.added', {"output_index":0,"item":{"id":output[0]['id'],"type":"message","role":"assistant","content":[]}})
            for index, chunk in enumerate(chunks):
                emit('response.output_text.delta', {"output_index":0,"item_id":output[0]['id'],"content_index":0,"delta":chunk})
                if index == 3 and not self.server.stream_gate.wait(timeout=60):
                    raise AssertionError('Streaming fixture held the response too long')
                time.sleep(.1)
        self.wfile.write(('event: response.completed\ndata: '+json.dumps({"response":{"status":"completed","output":output,"usage":{"input_tokens":100,"output_tokens":40,"input_tokens_details":{"cached_tokens":60},"output_tokens_details":{"reasoning_tokens":10}}}})+'\n\n').encode())

    def do_GET(self):
        path = self.path.split('?', 1)[0]
        inspector = ROOT / 'clients/inspector/dist'
        if path == '/architecture' or (not (Path(self.directory) / path.lstrip('/')).exists() and (inspector / path.lstrip('/')).is_file()):
            original = self.directory
            self.directory = str(inspector)
            if path == '/architecture':
                self.path = '/index.html'
            try:
                super().do_GET()
            finally:
                self.directory = original
            return
        if self.path.split('?', 1)[0] in ['/context', '/sessions', '/settings']:
            self.path = '/index.html'
        if self.path.startswith('/foundation.html'):
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.end_headers()
            html = (ROOT / 'clients/web/dist/index.html').read_text().replace('<head>', '<head>'+BOOTSTRAP)
            self.wfile.write(html.encode())
            return
        if self.path.startswith('/models?') or self.path == '/wham/usage':
            assert self.headers.get('Authorization') == 'Bearer fixture-access'
            assert self.headers.get('ChatGPT-Account-Id') == 'fixture-account'
            if self.path.startswith('/models?'):
                data = {"models": [{"slug": "fixture-model", "display_name": "Fixture", "visibility": "list", "priority": 0, "supported_reasoning_levels": []}]}
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
        server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Assets, directory=str(ROOT / 'clients/web/dist')))
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
command = "proteus-reference-worker"
[components.model.exports.model.custom-model]
[components.model.exports.workflow."coding.single_loop"]
[components.model.exports.tool."reference.tools"]
[components.model.exports.policy.allow_all]
[modules]
workflow = "coding.single_loop"
policy = "allow_all"
[tools]
enabled = ["update_plan"]
[module_config.model.custom-model]
implementation = "openai_codex"
base_url = ''' + json.dumps(web) + '\nquota_url = ' + json.dumps(web + '/wham/usage') + '\nauth_file = ' + json.dumps(str(auth)) + '\n[event_log]\npath = ' + json.dumps(str(folder / 'events.jsonl')) + '\n')
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
                reduced_motion = int(os.environ.get('PROTEUS_TEST_REDUCED_MOTION', '0'))
                capabilities = {'capabilities': {'alwaysMatch': {'browserName': 'firefox', 'moz:firefoxOptions': {'args': ['-headless'], 'prefs': {'network.proxy.type': 0, 'ui.prefersReducedMotion': reduced_motion}}}}}
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
                if '--inspector-only' in sys.argv:
                    check_architecture(command, js, wait_for, web, origin)
                    return
                check_extensions(command, js, wait_for, web, origin, loaded)
                check_selects(command, js, wait_for)
                check_panels(command, js, wait_for)
                check_usage(command, js, wait_for)
                check_layout(command, js, wait_for)
                if '--panels-only' in sys.argv:
                    print('PASS: extensions, custom selectors, owned file panels, motion and layout', flush=True)
                    return
                screenshot = request(url + '/screenshot')['value']
                Path('/tmp/proteus-ui-extensions.png').write_bytes(base64.b64decode(screenshot))
                check_architecture(command, js, wait_for, web, origin)
                check_session(command, js, wait_for, web, origin, loaded)
                check_queue(command, js, wait_for, server)
                check_live(command, js, wait_for, server)
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
