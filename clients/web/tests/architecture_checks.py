"""Built Inspector, real snapshot, node selection and responsive navigation."""
import base64
from pathlib import Path
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin):
    command('/url', {'url': web + '/architecture?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return !!document.querySelector('[data-node-id=\"slot:workflow\"]')"), 'Inspector graph did not mount from the real topology API')
    assert js("return !document.querySelector('script[src*=mermaid], .mermaid-map')"), 'Interactive graph still depends on Mermaid rendering'
    # Real pointer click must select a node without moving it before click lands.
    node = command('/element', {'using': 'css selector', 'value': '[data-node-id="slot:workflow"]'})
    command('/element/' + next(iter(node.values())) + '/click', {})
    assert js("return document.querySelector('.graph-node.selected').dataset.nodeId === 'slot:workflow' && document.querySelector('.graph-details').textContent.includes('coding.single_loop')")
    assert js("return document.querySelectorAll('.graph-connections > path.selected').length > 0 && document.querySelectorAll('.graph-node.dimmed').length > 0"), 'Selected node did not highlight neighbors'
    js("document.querySelector('[data-related-id=\"module:workflow:coding.single_loop\"]').click()")
    assert js("return document.querySelector('[data-scope=modules]').getAttribute('aria-pressed') === 'true' && document.querySelector('.graph-node.selected').dataset.nodeId === 'module:workflow:coding.single_loop'"), 'Related module did not switch map scope'
    js("const q=document.querySelector('.graph-toolbar input[type=search]');q.value='update_plan';q.dispatchEvent(new Event('input'));document.querySelector('.graph-search-results button').click()")
    assert js("return document.querySelector('.graph-node.selected').dataset.nodeId === 'tool:update_plan' && !!document.querySelector('.graph-schema pre')"), 'Search did not reveal tool details across scopes'
    js("const q=document.querySelector('.graph-toolbar input[type=search]');q.value='no-such-object-78623';q.dispatchEvent(new Event('input'))")
    assert js("return document.querySelector('.graph-search-results').textContent.includes('Ничего не найдено')")
    js("document.querySelector('.graph-viewport').focus();window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}))")
    assert js("return !document.querySelector('.graph-node.selected') && document.querySelector('.graph-search-results').hidden")
    before = js("return parseInt(document.querySelector('.graph-zoom').textContent)")
    js("document.querySelector('.graph-zoom-in').click()")
    assert js("return parseInt(document.querySelector('.graph-zoom').textContent)") > before
    before = js("return document.querySelector('.graph-stage').style.transform")
    js("document.querySelector('.graph-viewport').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}))")
    assert js("return document.querySelector('.graph-stage').style.transform") != before
    js("document.querySelector('[data-scope=assembly]').click()")
    command('/execute/async', {'script': 'requestAnimationFrame(()=>requestAnimationFrame(()=>arguments[arguments.length-1](null)))', 'args': []})
    Path('/tmp/proteus-architecture-ux.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("[...document.querySelectorAll('.architecture-tabs button')].find(b=>b.textContent==='Каталог сборки').click()")
    assert js("return !document.querySelector('.architecture-catalog').hidden && document.querySelector('.architecture-catalog').textContent.includes('update_plan')")
    js("document.querySelector('.architecture-tabs button').click()")
    for width in [900, 390]:
        command('/window/rect', {'width': width, 'height': 950})
        wait_for(lambda: js("return document.documentElement.scrollWidth <= innerWidth + 1"), 'Inspector overflows the narrow viewport')
        js("document.querySelector('.graph-controls button:last-child').click()")
        wait_for(lambda: js("return document.querySelector('.topology-explorer.fullscreen').getBoundingClientRect().width <= innerWidth"), 'Fullscreen graph is outside the window')
        js("window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}))")
        assert js("return !document.querySelector('.topology-explorer.fullscreen')")
    command('/window/rect', {'width': 1440, 'height': 1000})
    for _ in range(2):
        js("document.querySelector('[data-node-id=\"slot:workflow\"]').click();document.querySelector('.architecture-page .toolbar-actions button:last-child').click()")
        wait_for(lambda: js("return !document.querySelector('.graph-node.selected') && !!document.querySelector('[data-node-id=\"slot:workflow\"]') && document.querySelectorAll('.graph-toolbar').length === 1"), 'Refresh did not reset selection or duplicated graph mounts')
    command('/url', {'url': web + '/?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return !!document.querySelector('.composer textarea') && document.querySelector('.connection-badge')?.classList.contains('completed')"), 'Chat failed after Inspector navigation')
    print('PASS: Inspector graph API/selection/links/search/zoom/keyboard/catalog/resize/fullscreen', flush=True)
