"""Image input across the built app, HTTP admission, workflow and provider."""
import base64
import json
from pathlib import Path
import subprocess
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[4]
PNG = ROOT / 'crates/proteus-core/tests/fixtures/pixel.png'


def check_paste_and_drop(js, wait_for):
    """Clipboard images and files dropped over the chat use the picker path."""
    js("const bytes=Uint8Array.from(atob(" + json.dumps(base64.b64encode(PNG.read_bytes()).decode()) + "),c=>c.charCodeAt(0));"
       "window.transfer=(name='pasted.png',type='image/png',text='')=>{const t=new DataTransfer();t.items.add(new File([bytes],name,{type}));if(text)t.setData('text/plain',text);return t};"
       "window.fire=(target,event)=>{document.querySelector(target).dispatchEvent(event);return event.defaultPrevented}")
    previews = lambda: js('return document.querySelectorAll(".attachment-preview").length')
    remove = lambda: js("document.querySelector('.attachment-preview button').click()")
    # Firefox drops files from a synthetic ClipboardEvent init, so the
    # transfer is attached the way a real paste exposes it.
    paste = "Object.defineProperty(new ClipboardEvent('paste',{bubbles:true,cancelable:true}),'clipboardData',{value:transfer(%s)})"
    assert not js("return fire('.composer-input textarea'," + paste % "'cells.png','image/png','A1\\tB1'" + ")"), 'Spreadsheet text paste became an image'
    assert previews() == 0, 'Text paste attached its image rendition'
    assert js("return fire('.composer-input textarea'," + paste % "" + ")"), 'Image paste reached the textarea'
    wait_for(lambda: previews() == 1, 'Pasted image missing')
    remove()
    drag = lambda kind, args='': "new DragEvent('%s',{dataTransfer:transfer(%s),bubbles:true,cancelable:true})" % (kind, args)
    js("fire('.results-panel'," + drag('dragenter') + ")")
    assert js("return document.querySelector('.composer').classList.contains('dragging-files')"), 'Drag over the chat is not shown'
    js("fire('.results-panel'," + drag('dragleave') + ")")
    assert not js("return document.querySelector('.composer').classList.contains('dragging-files')"), 'Drag indicator stayed after leaving'
    js("fire('.results-panel'," + drag('dragenter') + ")")
    assert js("return fire('.results-panel'," + drag('drop') + ")"), 'Drop opened the file in the webview'
    wait_for(lambda: previews() == 1, 'Dropped image missing')
    assert not js("return document.querySelector('.composer').classList.contains('dragging-files')"), 'Drag indicator stayed after drop'
    # Force the overlap that used to restore an explicitly removed attachment.
    js("const original=Blob.prototype.arrayBuffer;window.originalImageRead=original;Blob.prototype.arrayBuffer=function(){const read=original.call(this);return this.name==='pending.png'?new Promise(resolve=>window.releaseImageRead=()=>read.then(resolve)):read};fire('.results-panel',"+drag('drop',"'pending.png','image/png'")+")")
    wait_for(lambda: js("return typeof window.releaseImageRead==='function'"), 'Deferred image read did not start')
    remove()
    assert previews() == 0, 'Previous image could not be removed during the next read'
    js("window.releaseImageRead();Blob.prototype.arrayBuffer=window.originalImageRead")
    wait_for(lambda: previews() == 1, 'New image was not merged into the current draft')
    assert js("return document.querySelector('.attachment-preview span').textContent==='pending.png'"), 'Deleted attachment was resurrected by a late read'
    remove()
    js("fire('.results-panel'," + drag('drop', "'notes.txt','text/plain'") + ")")
    wait_for(lambda: 'notes.txt: поддерживаются' in js("return document.querySelector('.attachment-error')?.textContent||''"), 'Unsupported drop was not explained')
    assert previews() == 0, 'Unsupported drop attached a file'


def run(command, js, wait_for, server, web, origin):
    def attach():
        element = command('/element', {'using': 'css selector', 'value': '.composer-attach input[type=file]'})
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
    check_paste_and_drop(js, wait_for)
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
    print('PASS: file selection/removal; clipboard paste; drop over the chat; image+text; image-only queue/edit; authorized preview; cold UI reload; provider history; compact layout; native WebKitGTK', flush=True)
