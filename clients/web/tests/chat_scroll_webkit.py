#!/usr/bin/env python3
"""WebKitGTK regression for reading near the bottom of a streaming chat.

Run after ``trunk build`` and ``cargo build -p proteus-core -p proteus-reference-worker``.
Requires Python GI, GTK3, WebKit2 4.1 and Xvfb. Use ``--wayland`` when Xvfb is
unavailable and a Wayland session is running. The app-server and model fixture
are local; no account or existing Proteus session is used.
"""

import atexit
import argparse
from functools import partial
from http.server import ThreadingHTTPServer
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import threading
import time

import extensions_browser as fixture
from extensions_browser import Assets, ROOT, stop, urlencode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wayland', action='store_true')
    parser.add_argument('--history', type=int, default=240)
    args = parser.parse_args()
    assert args.history >= 2, 'History needs at least two messages'
    fixture.BOOTSTRAP = fixture.BOOTSTRAP.replace('length: 240', 'length: ' + str(args.history))
    display = None
    if '--wayland' in sys.argv:
        os.environ['GDK_BACKEND'] = 'wayland'
    else:
        display = subprocess.Popen(
            ['Xvfb', '-displayfd', '1', '-screen', '0', '1440x1000x24', '-nolisten', 'tcp'],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        def stop_display():
            if display.poll() is None:
                display.terminate()
                display.wait(timeout=5)
        atexit.register(stop_display)
        number = display.stdout.readline().strip()
        assert number.isdecimal(), 'Xvfb did not start; use --wayland in a Wayland session'
        os.environ.update(DISPLAY=':' + number, GDK_BACKEND='x11')
        os.environ.pop('WAYLAND_DISPLAY', None)
        os.environ['WEBKIT_DISABLE_COMPOSITING_MODE'] = '1'
    os.environ['WEBKIT_DISABLE_DMABUF_RENDERER'] = '1'

    import gi
    gi.require_version('Gtk', '3.0')
    gi.require_version('WebKit2', '4.1')
    from gi.repository import GLib, Gtk, WebKit2

    with tempfile.TemporaryDirectory(prefix='proteus-chat-scroll-') as temporary:
        folder = Path(temporary)
        server = ThreadingHTTPServer(
            ('127.0.0.1', 0), partial(Assets, directory=str(ROOT / 'clients/web/dist')),
        )
        server.model_inputs = []
        server.model_requests = 2  # The fixture's third response streams 32 paragraphs.
        server.model_gate = threading.Event()
        server.model_gate.set()
        server.stream_gate = threading.Event()  # Pause after paragraph 3.
        threading.Thread(target=server.serve_forever, daemon=True).start()
        web = f'http://127.0.0.1:{server.server_port}'
        auth = folder / 'auth.json'
        auth.write_text(json.dumps({
            'access_token': 'fixture-access', 'refresh_token': 'fixture-refresh',
            'account_id': 'fixture-account', 'expires_at': 9000000000,
        }))
        config = folder / 'fixture.toml'
        config.write_text('''active_provider = "subscription"
[providers.subscription]
provider = "custom-model"
model = "fixture-model"
[components.model]
command = "proteus-reference-worker"
[components.model.exports.model.custom-model]
[components.model.exports.workflow."coding.single_loop"]
[components.model.exports.policy.allow_all]
[modules]
workflow = "coding.single_loop"
policy = "allow_all"
[module_config.model.custom-model]
implementation = "openai_codex"
base_url = ''' + json.dumps(web) + '\nquota_url = ' + json.dumps(web + '/wham/usage')
                          + '\nauth_file = ' + json.dumps(str(auth))
                          + '\n[event_log]\npath = ' + json.dumps(str(folder / 'events.jsonl')) + '\n')
        env = os.environ.copy()
        env.pop('PROTEUS_CONFIG_PATH', None)
        env.update(PATH=str(ROOT / 'target/debug') + ':' + env['PATH'],
                   PROTEUS_CONFIG_HOME=str(folder / 'config'),
                   XDG_CONFIG_HOME=str(folder / 'settings'),
                   XDG_DATA_HOME=str(folder / 'data'))
        backend = None
        window = None
        with (folder / 'backend.log').open('w+') as log:
            try:
                backend = subprocess.Popen([
                    str(ROOT / 'target/debug/proteus'), '--config', str(config),
                    '--cwd', str(folder), 'server', 'http', '--port', '0',
                    '--token', 'scroll-fixture', '--ready-stdout', '--allow-origin', web,
                ], env=env, stdout=subprocess.PIPE, stderr=log, text=True,
                    start_new_session=True)
                origin = None
                deadline = time.monotonic() + 35
                while time.monotonic() < deadline and backend.poll() is None:
                    if select.select([backend.stdout], [], [], 0.1)[0]:
                        line = backend.stdout.readline()
                        if line.startswith('{'):
                            record = json.loads(line)
                            if record.get('type') == 'http_ready':
                                origin = record['origin']
                                break
                assert origin, 'Fixture app-server did not start: ' + log.read()

                window = Gtk.Window(title='Proteus chat scroll regression')
                window.set_default_size(1440, 1000)
                view = WebKit2.WebView()
                window.add(view)
                window.show_all()
                state = {'stage': 0, 'pending': False, 'detached_top': None}
                errors = []

                def fail(message, value=None):
                    errors.append(message + (': ' + json.dumps(value, ensure_ascii=False) if value is not None else ''))
                    Gtk.main_quit()

                def evaluate(script, callback):
                    state['pending'] = True

                    def done(webview, task, unused):
                        state['pending'] = False
                        try:
                            raw = webview.evaluate_javascript_finish(task).to_json(0)
                            callback(json.loads(raw) if raw else None)
                        except Exception as error:
                            fail('WebKit JavaScript failed: ' + str(error))

                    view.evaluate_javascript(script, -1, None, None, None, done, None)

                def handle(value):
                    stage = state['stage']
                    if stage == 0 and value:
                        state['stage'] = 1
                        evaluate('''(() => {
                            const r = document.querySelector('.results-panel');
                            window.scrollEvents = 0;
                            r.addEventListener('scroll', () => scrollEvents++);
                            const draft = document.querySelector('.composer textarea');
                            draft.value = 'Проверить прокрутку';
                            draft.dispatchEvent(new Event('input', {bubbles:true}));
                            requestAnimationFrame(() => document.querySelector('.composer-submit').click());
                            return true;
                        })()''', lambda _: None)
                    elif stage == 1 and value:
                        state['stage'] = 2
                        evaluate('''(() => {
                            const r = document.querySelector('.results-panel');
                            r.scrollTop = r.scrollHeight;
                            return true;
                        })()''', lambda _: None)
                    elif stage == 2 and value and value['atBottom'] and value['sticky'] and value['scrollEvents']:
                        state['stage'] = 3
                        evaluate('''(() => {
                            const r = document.querySelector('.results-panel');
                            window.scrollEvents = 0;
                            // The upward wheel can arrive while a previous bottom scroll
                            // event is still queued. Move only one pixel: this remains
                            // inside the normal 4px bottom tolerance.
                            r.dispatchEvent(new WheelEvent('wheel', {deltaY:-0.5, bubbles:true}));
                            r.dispatchEvent(new Event('scroll', {bubbles:true}));
                            r.scrollTop = r.scrollHeight - r.clientHeight - 1;
                            const y=r.getBoundingClientRect().top;
                            window.readingAnchor=[...r.querySelectorAll('[data-transcript-row]')].find(n=>n.getBoundingClientRect().top<=y && n.getBoundingClientRect().bottom>y);
                            return {top:readingAnchor.getBoundingClientRect().top-y, max:r.scrollHeight-r.clientHeight};
                        })()''', lambda result: state.update(detached_top=result['top']))
                    elif stage == 3 and value and value['scrollEvents']:
                        if value['sticky'] or value['anchorTop'] is None or abs(value['anchorTop'] - state['detached_top']) > 1:
                            fail('Upward reading gesture snapped to bottom', value)
                            return
                        state['stage'] = 4
                        server.stream_gate.set()
                    elif stage == 4 and value and value['settled']:
                        if value['sticky'] or value['anchorTop'] is None or abs(value['anchorTop'] - state['detached_top']) > 2:
                            fail('New streamed content pulled the reader from history', value)
                            return
                        state['stage'] = 5
                        evaluate('''(() => {
                            const r = document.querySelector('.results-panel');
                            r.dispatchEvent(new WheelEvent('wheel', {deltaY:120, bubbles:true}));
                            r.scrollTop = r.scrollHeight;
                            return true;
                        })()''', lambda _: None)
                    elif stage == 5 and value and value['sticky'] and value['atBottom']:
                        Gtk.main_quit()

                def poll():
                    if errors or state['pending']:
                        return not errors
                    stage = state['stage']
                    if stage == 0:
                        script = "!!document.querySelector('.results-panel')?.textContent.includes('Сохранённое сообщение " + str(args.history - 1) + "') && document.querySelector('.connection-badge')?.classList.contains('completed')"
                    elif stage == 1:
                        script = "document.querySelector('.results-panel')?.textContent.includes('Абзац 3:') && !!document.querySelector('.composer-stop')"
                    else:
                        script = '''(() => {
                            const r = document.querySelector('.results-panel');
                            const max = r.scrollHeight-r.clientHeight;
                            return {anchorTop:window.readingAnchor?.isConnected ? readingAnchor.getBoundingClientRect().top-r.getBoundingClientRect().top : null,top:r.scrollTop,max,sticky:r.classList.contains('sticky-bottom'),
                                atBottom:max-r.scrollTop<=1,scrollEvents:scrollEvents,
                                settled:!document.querySelector('.composer-stop') && r.textContent.includes('Абзац 31:')};
                        })()'''
                    evaluate(script, handle)
                    return True

                def timeout():
                    fail('Chat scroll regression timed out at stage ' + str(state['stage']))
                    return False

                GLib.timeout_add(100, poll)
                GLib.timeout_add_seconds(45, timeout)
                view.load_uri(web + '/foundation.html?' + urlencode({
                    'server': origin, 'token': 'scroll-fixture',
                }))
                Gtk.main()
                assert not errors, '\n'.join(errors)
                assert state['stage'] == 5, 'Chat scroll regression ended at stage ' + str(state['stage'])
                print('PASS: WebKit near-bottom upward reading survives queued scroll and streaming; downward return resumes following', flush=True)
            finally:
                server.stream_gate.set()
                if window is not None:
                    window.destroy()
                stop(backend)
                server.shutdown()
    if display is not None:
        stop_display()
        atexit.unregister(stop_display)


if __name__ == '__main__':
    main()
