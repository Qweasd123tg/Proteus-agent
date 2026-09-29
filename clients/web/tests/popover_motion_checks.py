#!/usr/bin/env python3
"""Production popover timelines in native WebKitGTK or isolated Firefox.

Use --cost to report synchronous close costs for 10/30/100/300 menu options.
"""
from functools import partial
from http.server import ThreadingHTTPServer
from pathlib import Path
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import threading

import popovers_webkit as harness

harness.PAGE = Path(__file__).with_name('popover_motion_fixture.html').read_text()
if '--cost' in sys.argv:
    harness.PAGE = harness.PAGE.replace('<script type="module">', '<script>window.snapshotCostMode=true</script><script type="module">', 1)


def firefox(reduced=False):
    from extensions_browser import request, wait_for, stop

    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(harness.Assets, directory=str(harness.ROOT)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with socket.socket() as reserve:
        reserve.bind(('127.0.0.1', 0))
        port = reserve.getsockname()[1]
    endpoint = f'http://127.0.0.1:{port}'
    binary = shutil.which('geckodriver')
    if not binary:
        binary = str(sorted((Path.home()/'.cache/selenium/geckodriver/linux64').glob('*/geckodriver'))[-1])
    session = None
    with tempfile.TemporaryFile(mode='w+') as log:
        driver = subprocess.Popen([binary, '--port', str(port)], stdout=log, stderr=log, start_new_session=True)
        try:
            def ready():
                try:
                    return request(endpoint+'/status')['value']['ready']
                except OSError:
                    return False
            wait_for(ready, 'geckodriver startup')
            options = {'args': ['-headless'], 'prefs': {'ui.prefersReducedMotion': int(reduced)}}
            session = request(endpoint+'/session', 'POST', {'capabilities': {'alwaysMatch': {'browserName': 'firefox', 'moz:firefoxOptions': options}}})['value']['sessionId']
            url = endpoint+'/session/'+session
            request(url+'/window/rect', 'POST', {'width': 1440, 'height': 1000})
            request(url+'/url', 'POST', {'url': f'http://127.0.0.1:{server.server_port}/probe.html'})
            results = []
            def probe():
                value = request(url+'/execute/sync', 'POST', {'script': 'return window.probe ? window.probe() : null', 'args': []})['value']
                if value:
                    results.extend(value)
                    return True
            wait_for(probe, 'popover motion timeline')
            assert len(results) == 4, results
            print('PASS: Firefox popover motion:', json.dumps(results))
        finally:
            if session:
                request(endpoint+'/session/'+session, 'DELETE')
            stop(driver)
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    if '--firefox' in sys.argv:
        firefox(reduced='--reduced' in sys.argv)
    else:
        harness.main(label='WebKitGTK popover close cost' if '--cost' in sys.argv else 'WebKitGTK popover motion')
