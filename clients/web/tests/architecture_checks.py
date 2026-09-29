"""Built Inspector, real snapshot, node selection and responsive navigation."""
import base64
from pathlib import Path
from urllib.parse import urlencode
from select_checks import run as check_selects


def run(command, js, wait_for, web, origin):
    command('/url', {'url': web + '/architecture?' + urlencode({'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("return !!document.querySelector('[data-node-id=\"slot:workflow\"]')"), 'Inspector graph did not mount from the real topology API')
    check_selects(command, js, wait_for)
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
    assert js(r"""
        const viewport = document.querySelector('.graph-viewport'), stage = document.querySelector('.graph-stage');
        const zoom = document.querySelector('.graph-zoom'), before = stage.style.transform;
        const start = before.match(/translate\(([-\d.]+)px, ([-\d.]+)px\)/);
        const stageChanges = new MutationObserver(() => {}), zoomChanges = new MutationObserver(() => {});
        stageChanges.observe(stage, {attributes: true, attributeFilter: ['style']});
        zoomChanges.observe(zoom, {childList: true});
        viewport.setPointerCapture = () => {};
        const point = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 37, clientX: x, clientY: y});
        viewport.dispatchEvent(point('pointerdown', 40, 40));
        for (let index = 0; index < 80; index++) viewport.dispatchEvent(point('pointermove', 40 + index, 40 + index / 2));
        const queued = stage.style.transform === before && stageChanges.takeRecords().length === 0;
        viewport.dispatchEvent(point('pointerup', 220, 100));
        const end = stage.style.transform.match(/translate\(([-\d.]+)px, ([-\d.]+)px\)/);
        const result = queued && end && Math.abs(Number(end[1]) - Number(start[1]) - 180) < .01
          && Math.abs(Number(end[2]) - Number(start[2]) - 60) < .01
          && stageChanges.takeRecords().length === 1 && zoomChanges.takeRecords().length === 0;
        stageChanges.disconnect(); zoomChanges.disconnect(); delete viewport.setPointerCapture;
        return !!result;
    """), 'Graph drag did not batch moves or flush the final pointer position'
    assert js(r"""
        const viewport = document.querySelector('.graph-viewport'), stage = document.querySelector('.graph-stage');
        const before = stage.style.transform, start = Number(before.match(/scale\(([-\d.]+)\)/)[1]);
        const rect = viewport.getBoundingClientRect(), observer = new MutationObserver(records => probe.writes += records.length);
        const probe = {stage, observer, writes: 0, expected: Math.min(2.5, start * Math.exp(.004 * 40))};
        observer.observe(stage, {attributes: true, attributeFilter: ['style']});
        for (let index = 0; index < 40; index++) viewport.dispatchEvent(new WheelEvent('wheel', {
          bubbles: true, cancelable: true, deltaY: -1, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2,
        }));
        probe.queued = stage.style.transform === before && observer.takeRecords().length === 0;
        window.__graphFrameProbe = probe;
        return probe.queued;
    """), 'Graph wheel burst wrote a transform before the animation frame'
    wait_for(lambda: js("const p=window.__graphFrameProbe;return Math.abs(Number(p.stage.style.transform.match(/scale\\(([-\\d.]+)\\)/)[1])-p.expected)<.0001"), 'Graph wheel burst did not apply its accumulated zoom')
    assert js("const p=window.__graphFrameProbe;const result=p.writes+p.observer.takeRecords().length===1;p.observer.disconnect();delete window.__graphFrameProbe;return result"), 'Graph wheel burst wrote more than one transform per frame'
    assert command('/execute/async', {'script': """
        const done = arguments[arguments.length - 1];
        import('/graph/view.js').then(({mountTopologyGraph}) => {
          const root = document.createElement('div');
          root.style.width = '400px'; root.style.height = '300px'; document.body.append(root);
          const source = JSON.stringify({profile: 'test', cwd: '/', config_files: [], module_epoch: 1,
            permission_mode: 'ask', slots: [], modules: [], tools: [], edges: []});
          const dispose = mountTopologyGraph(root, source);
          const stage = root.querySelector('.graph-stage'), viewport = root.querySelector('.graph-viewport');
          const rect = viewport.getBoundingClientRect(), before = stage.style.transform;
          viewport.dispatchEvent(new WheelEvent('wheel', {bubbles: true, cancelable: true, deltaY: -1,
            clientX: rect.left + 50, clientY: rect.top + 50}));
          dispose();
          requestAnimationFrame(() => requestAnimationFrame(() => {
            const result = root.childElementCount === 0 && stage.style.transform === before;
            root.remove(); done(result);
          }));
        }).catch(error => done(String(error)));
    """, 'args': []}), 'Disposed graph applied a pending transform'
    js("document.querySelector('[data-scope=assembly]').click()")
    command('/execute/async', {'script': 'requestAnimationFrame(()=>requestAnimationFrame(()=>arguments[arguments.length-1](null)))', 'args': []})
    Path('/tmp/proteus-architecture-ux.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("[...document.querySelectorAll('.architecture-tabs button')].find(b=>b.textContent==='Каталог сборки').click()")
    assert js("return !document.querySelector('.architecture-catalog').hidden && document.querySelector('.architecture-catalog').textContent.includes('update_plan')")
    js("document.querySelector('.architecture-tabs button').click()")
    for width in [900, 390]:
        command('/window/rect', {'width': width, 'height': 950})
        wait_for(lambda: js("return document.documentElement.scrollWidth <= Math.max(innerWidth,860) + 1"), 'Inspector overflows the narrow viewport')
        assert js("const side=document.querySelector('.inspector-sidebar').getBoundingClientRect(),main=document.querySelector('.inspector-main').getBoundingClientRect(),graph=document.querySelector('.graph-viewport').getBoundingClientRect(),details=document.querySelector('.graph-details').getBoundingClientRect();return side.right<=main.left && side.top===main.top && getComputedStyle(document.querySelector('.inspector-sidebar')).flexDirection==='column' && (document.querySelector('.graph-details').hidden || details.left>=graph.right)"), 'Inspector switched to vertical panes'
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
    print('PASS: Inspector graph API/selection/links/search/zoom/keyboard/frame batching/teardown/catalog/resize/fullscreen', flush=True)
