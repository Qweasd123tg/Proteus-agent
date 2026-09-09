#!/usr/bin/env python3
"""Real Firefox + built Leptos + isolated fake-model app-server. No live account.

Requires Firefox and geckodriver (PATH or GECKODRIVER). Only stdlib Python.
Run after trunk build and cargo build -p proteus-core -p proteus-reference-worker.
"""
import base64
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

    def do_GET(self):
        if self.path.startswith('/fixture/'):
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
        config = folder / 'fake.toml'
        config.write_text('''active_provider = "fake"
[profile]
name = "extensions-smoke"
[providers.fake]
provider = "fake"
model = "fake-model"
[components.model]
command = "proteus-reference-worker"
[components.model.exports.model.fake]
[module_config.model.fake]
implementation = "fake"
[event_log]
path = ''' + json.dumps(str(folder / 'events.jsonl')) + '\n')
        server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Assets, directory=str(ROOT / 'clients/web/dist')))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        web = f'http://127.0.0.1:{server.server_port}'
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
                capabilities = {'capabilities': {'alwaysMatch': {'browserName': 'firefox', 'moz:firefoxOptions': {'args': ['-headless'], 'prefs': {'network.proxy.type': 0}}}}}
                session = request(endpoint + '/session', 'POST', capabilities)['value']['sessionId']
                url = endpoint + '/session/' + session
                def command(path, body):
                    return request(url + path, 'POST', body)['value']
                def js(script):
                    return command('/execute/sync', {'script': script, 'args': []})
                def loaded():
                    return js("return document.querySelector('[data-extension-id=agent-info] .extension-panel-content')?.shadowRoot?.textContent.includes('extensions-smoke')")
                command('/window/rect', {'width': 1440, 'height': 1000})
                command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                wait_for(loaded, 'Actual Leptos transport did not deliver authenticated /config to the extension')
                js("if (!document.querySelector('.info-panel.open')) document.querySelector('.info-panel-header button').click(); document.querySelector('.extension-manager').open = true; document.querySelector('[data-extension-choice=notes] input').click()")
                wait_for(lambda: js("return !!document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')"), 'Notes did not mount')
                js("const area = document.querySelector('[data-extension-id=notes] .extension-panel-content').shadowRoot.querySelector('textarea'); area.value = 'Моя заметка'; area.dispatchEvent(new Event('input')); document.querySelector('[aria-label=\"Выше: Заметки\"]').click()")
                assert js("return document.querySelector('.extension-panels').firstElementChild.dataset.extensionId") == 'notes'
                command('/refresh', {})
                wait_for(loaded, 'Reload did not restore agent extension')
                wait_for(lambda: js("return document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')?.value === 'Моя заметка'"), 'Notes did not survive reload')
                assert js("return document.querySelector('.extension-panels').firstElementChild.dataset.extensionId") == 'notes'
                js("document.querySelector('.extension-manager').open = true; document.querySelector('.extension-install input').value = location.origin + '/fixture/extension.json'; document.querySelector('.extension-install').requestSubmit()")
                wait_for(lambda: js('return window.externalMounted === 1'), 'External package did not mount without rebuild')
                js("document.querySelector('[data-extension-choice=external-test] input').click()")
                assert js('return window.externalAborted === 1 && window.externalDisposed === 1')
                js("document.querySelector('[data-extension-choice=external-test] input').click()")
                wait_for(lambda: js('return window.externalMounted === 2'), 'Re-enabled package did not mount')
                js("document.querySelector('[data-extension-id=external-test] .extension-panel-title').click()")
                assert js('return window.externalAborted === 2 && window.externalDisposed === 2')
                js("document.querySelector('[data-extension-id=external-test] .extension-panel-title').click()")
                wait_for(lambda: js('return window.externalMounted === 3'), 'Expanded panel did not remount')
                js("document.querySelector('.topnav a[href=\"/context\"]').click()")
                wait_for(lambda: js("return !document.querySelector('.extension-host')"), 'SPA navigation did not unmount panels')
                assert js('return window.externalAborted === 3 && window.externalDisposed === 3')
                js("document.querySelector('.topnav a[href=\"/\"]').click()")
                wait_for(loaded, 'Return to chat did not reconnect panel')
                wait_for(lambda: js('return window.externalMounted === 4'), 'Return to chat did not restore external panel')
                js("document.querySelector('[data-extension-id=external-test] .extension-panel-title').click()")
                js("document.querySelector('.extension-manager').open = true; document.querySelector('.extension-install input').value = location.origin + '/fixture/slow/extension.json'; document.querySelector('.extension-install').requestSubmit()")
                wait_for(lambda: js('return window.slowMounts === 1'), 'Async fixture did not mount')
                js("document.querySelector('[data-extension-choice=slow-test] input').click(); document.querySelector('[data-extension-choice=slow-test] input').click()")
                wait_for(lambda: js('return window.slowMounts === 2'), 'Second async instance did not mount')
                js('window.finishSlowMount()')
                wait_for(lambda: js('return window.slowDisposals === 1'), 'Late disposer was lost')
                assert js("return document.querySelector('[data-extension-id=slow-test] .extension-panel-content').shadowRoot.textContent.includes('current panel')"), 'Old async disposer erased new panel'
                js("document.querySelector('[aria-label=\"Убрать: Медленная панель\"]').click()")
                # No runtime mutation or provider request is needed to use notes in another host.
                command('/url', {'url': web + '/standalone.html'})
                wait_for(lambda: js("return document.querySelector('[data-extension-id=notes] .extension-panel-content')?.shadowRoot?.querySelector('textarea')?.value === 'Моя заметка'"), 'Independent host did not load notes')
                wait_for(lambda: js("return document.querySelector('[data-extension-id=agent-info] .extension-error')?.textContent.includes('agent.config.read')"), 'Missing service was not reported locally')
                assert js("return !!document.querySelector('[data-extension-id=notes] .extension-panel-content').shadowRoot.querySelector('textarea')")
                command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
                wait_for(loaded, 'Final client reload failed')
                for width in [900, 640, 390]:
                    command('/window/rect', {'width': width, 'height': 1000})
                    assert js('return document.documentElement.scrollWidth <= window.innerWidth'), f'Horizontal overflow at {width}px'
                command('/window/rect', {'width': 1440, 'height': 1000})
                screenshot = request(url + '/screenshot')['value']
                Path('/tmp/proteus-ui-extensions.png').write_bytes(base64.b64decode(screenshot))
                print('PASS: real Leptos + authenticated agent API; local notes and order survive reload; external package install and lifecycle; independent host without agent')
            except Exception:
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
