"""Image input across the built app, HTTP admission, workflow and provider."""
import base64
import json
from pathlib import Path
import subprocess
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[4]
PNG = ROOT / 'crates/proteus-core/tests/fixtures/pixel.png'


def run(command, js, wait_for, server, web, origin):
    def attach():
        element = command('/element', {'using': 'css selector', 'value': '.composer-attachments input[type=file]'})
        key = next(iter(element.values()))
        command('/element/'+key+'/value', {'text': str(PNG)})
        wait_for(lambda: js('return document.querySelectorAll(".attachment-preview").length===1'), 'Image preview missing')

    def send(text):
        js("const input=document.querySelector('.composer-input textarea');input.value="+json.dumps(text)+";input.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js("return !document.querySelector('.composer-submit').disabled"), 'Image draft did not enable send')
        js("document.querySelector('.composer-submit').click()")

    def ready(count):
        return js("return !document.querySelector('.composer-stop') && document.querySelector('.results-panel').textContent.includes("+json.dumps('Изображений: '+str(count))+")")

    attach()
    js("document.querySelector('.attachment-preview button').click()")
    assert js('return !document.querySelector(".attachment-preview")'), 'Removal left an attachment'
    attach()
    send('Что на картинке?')
    wait_for(lambda: ready(1), 'Image response missing')
    wait_for(lambda: js('return document.querySelector(".message-images img")?.naturalWidth===240'), 'Stored image did not render')
    first = server.model_inputs[0]['input']
    assert any([p['type'] for p in item.get('content', [])] == ['input_image', 'input_text'] for item in first), 'Image and text split into different canonical messages'
    assert js('return !document.querySelector(".attachment-preview")'), 'Sent image stayed in draft'

    # Editing a queued instruction must preserve its attached image.
    server.model_gate.clear()
    try:
        before = server.model_requests
        send('Уточни детали')
        wait_for(lambda: server.model_requests > before, 'Model did not start')
        attach()
        send('')
        wait_for(lambda: js('return document.querySelector(".queued-image-count")?.textContent.includes("1")'), 'Image-only queue row lost its label')
        js("document.querySelector('.queued-prompt-row button').click()")
        wait_for(lambda: js('return !!document.querySelector(".queued-prompt-editor textarea")'), 'Queue editor missing')
        js("const input=document.querySelector('.queued-prompt-editor textarea');input.value='Вторая картинка';input.dispatchEvent(new Event('input',{bubbles:true}))")
        wait_for(lambda: js('return !document.querySelector(".queue-editor-actions .primary").disabled'), 'Queue edit did not enable save')
        js("document.querySelector('.queue-editor-actions .primary').click()")
        wait_for(lambda: js('return !document.querySelector(".queued-prompt-editor") && document.querySelector(".queued-image-count")?.textContent.includes("1")'), 'Queue edit dropped image metadata')
    finally:
        server.model_gate.set()
    wait_for(lambda: ready(2), 'Edited queued image did not reach model')
    wait_for(lambda: js('return document.querySelectorAll(".message-images img").length===2 && [...document.querySelectorAll(".message-images img")].every(i=>i.naturalWidth===240)'), 'Queued image missing from transcript')
    command('/refresh', {})
    wait_for(lambda: js('return document.querySelectorAll(".message-images img").length===2 && [...document.querySelectorAll(".message-images img")].every(i=>i.naturalWidth===240)'), 'Reload lost stored images')
    send('После перезагрузки')
    wait_for(lambda: server.model_requests >= 4 and ready(2), 'Reloaded history did not reach model')
    command('/window/rect', {'width': 900, 'height': 850})
    wait_for(lambda: js('return innerWidth<1000'), 'Responsive viewport missing')
    assert js('return document.documentElement.scrollWidth<=innerWidth'), 'Image layout overflows compact viewport'
    Path('/tmp/proteus-images-ui.png').write_bytes(base64.b64decode(command('/screenshot', None)))

    url = web+'/?'+urlencode({'server': origin, 'token': 'extension-smoke'})
    subprocess.run(['python3', str(Path(__file__).with_name('images_webkit.py')), url], check=True, timeout=90)
    assert server.model_requests >= 5, 'Native WebKit did not invoke the provider'
    print('PASS: file selection/removal; image+text; image-only queue/edit; authorized preview; cold UI reload; provider history; compact layout; native WebKitGTK', flush=True)
