#!/usr/bin/env python3
"""Exercise image-only input in the actual built app under native WebKitGTK."""
import base64
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[4]
PNG = base64.b64encode((ROOT / 'crates/proteus-core/tests/fixtures/pixel.png').read_bytes()).decode()
SCRIPT = '''
(() => {
  if (window.imageProbe) return window.imageProbe;
  const input=document.querySelector('.composer-attachments input[type=file]');
  const ready=document.querySelector('[data-extension-id=agent-info] .extension-panel-content')?.shadowRoot?.textContent.includes('extensions-smoke');
  if (!input || !ready || document.querySelector('.composer-stop')) return null;
  window.imageProbe={stage:'loading'};
  (async()=>{
    const wait=async(test,label)=>{for(let n=0;n<160;n++){if(test())return;await new Promise(r=>setTimeout(r,250));}throw Error(label);};
    const bytes=Uint8Array.from(atob(PNG_DATA),c=>c.charCodeAt(0));
    const transfer=new DataTransfer();transfer.items.add(new File([bytes],'native.png',{type:'image/png'}));
    input.files=transfer.files;input.dispatchEvent(new Event('change',{bubbles:true}));
    await wait(()=>document.querySelector('.attachment-preview img')?.naturalWidth===240,'Native File.arrayBuffer/preview failed');
    const previous=document.querySelectorAll('.message-images img').length;
    const textarea=document.querySelector('.composer-input textarea');textarea.value='';textarea.dispatchEvent(new Event('input',{bubbles:true}));
    await wait(()=>!document.querySelector('.composer-submit').disabled,'Native image-only submit disabled');
    document.querySelector('.composer-submit').click();
    await wait(()=>document.querySelectorAll('.message-images img').length>previous && [...document.querySelectorAll('.message-images img')].every(i=>i.naturalWidth===240),'Native stored image missing');
    await wait(()=>!document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes('Изображений:'),'Native model response missing');
    window.imageProbe={ok:true,images:document.querySelectorAll('.message-images img').length};
  })().catch(error=>window.imageProbe={error:String(error)+'; previews='+document.querySelectorAll('.attachment-preview').length+'; '+(document.querySelector('.attachment-error')?.textContent||'')});
  return null;
})()
'''.replace('PNG_DATA', json.dumps(PNG))


def main(url):
    with tempfile.TemporaryFile(mode='w+') as log:
        display = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '1200x900x24', '-nolisten', 'tcp'], stdout=subprocess.PIPE, stderr=log, text=True)
        try:
            number = display.stdout.readline().strip()
            assert number.isdecimal(), 'Xvfb did not start'
            os.environ.update(DISPLAY=':'+number, GDK_BACKEND='x11', WEBKIT_DISABLE_COMPOSITING_MODE='1')
            os.environ.pop('WAYLAND_DISPLAY', None)
            import gi
            gi.require_version('Gtk', '3.0')
            gi.require_version('WebKit2', '4.1')
            from gi.repository import Gtk, WebKit2, GLib
            window = Gtk.Window()
            window.set_default_size(1200, 900)
            view = WebKit2.WebView()
            window.add(view)
            window.show_all()
            results, errors = [], []

            def done(view, task, data):
                try:
                    raw = view.evaluate_javascript_finish(task).to_json(0)
                    value = json.loads(raw) if raw else None
                    if value and value.get('error'):
                        errors.append(value['error'])
                        Gtk.main_quit()
                    elif value and value.get('ok'):
                        results.append(value)
                        Gtk.main_quit()
                except Exception as error:
                    errors.append(str(error))
                    Gtk.main_quit()

            def probe():
                view.evaluate_javascript(SCRIPT, -1, None, None, None, done, None)
                return not (results or errors)

            def timeout():
                errors.append('Native image probe timed out')
                Gtk.main_quit()
                return False

            GLib.timeout_add(500, probe)
            GLib.timeout_add_seconds(60, timeout)
            view.load_uri(url)
            try:
                Gtk.main()
            finally:
                window.destroy()
            assert not errors, '\n'.join(errors)
            assert results
            print('PASS: native WebKitGTK image-only input, File.arrayBuffer and stored preview', flush=True)
        finally:
            display.terminate()
            display.wait(timeout=5)


if __name__ == '__main__':
    main(sys.argv[1])
