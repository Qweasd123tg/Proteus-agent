"""Real assistant JSON fence, local interactions, invalid source and stream lifecycle."""
import json
import re
from pathlib import Path

SKILL = Path(__file__).resolve().parents[3] / 'configs/skills/interactive-response/SKILL.md'
SOURCE = re.search(r'```json-render\n([\s\S]*?)\n```', SKILL.read_text())[1]
FIXTURE = '\n\n```json-render\n' + SOURCE + '\n```\n\nТекст после визуализации.\n'


def run(command, js, wait_for):
    def ready():
        return js("return document.querySelectorAll('.role-assistant .markdown-interactive').length===1 && document.querySelectorAll('.jr-bar').length===2")
    wait_for(ready, 'JSON response did not render')
    assert js("return document.querySelector('.jr-bar-value').textContent==='80 с'"), 'Chart value lost'
    js("document.querySelectorAll('.jr-tab')[1].click();const search=document.querySelector('.jr-search');search.value='После';search.dispatchEvent(new Event('input',{bubbles:true}))")
    assert js("return document.querySelectorAll('.jr-tab')[1].getAttribute('aria-selected')==='true' && document.querySelectorAll('.jr-table tbody tr').length===1 && document.querySelector('.jr-table tbody').textContent.includes('35')"), 'Tabs / filter did not respond'
    js("document.querySelectorAll('.jr-tab')[1].dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true,cancelable:true}))")
    assert js("return document.querySelector('.jr-tab').getAttribute('aria-selected')==='true'"), 'Keyboard tab navigation failed'
    js("document.querySelector('.markdown-interactive').closest('.code-block').querySelector('.code-source').click()")
    assert js("return !document.querySelector('code.language-json-render').parentElement.hidden && document.querySelector('code.language-json-render').textContent.includes('summary')"), 'JSON source unavailable'
    assert js("return performance.getEntriesByType('resource').filter(r=>r.name.includes('json-render')).every(r=>new URL(r.name).origin===location.origin)"), 'Remote renderer dependency'
    # Production renderer through mutation queue: wait for final streamed content.
    js("window.jsonProbe=document.createElement('div');jsonProbe.className='message streaming-message';jsonProbe.innerHTML='<div class=code-block><div class=code-actions></div><pre><code class=language-json-render></code></pre></div>';jsonProbe.querySelector('code').textContent=" + json.dumps(SOURCE) + ";document.querySelector('.results-panel').append(jsonProbe)")
    assert js("return !jsonProbe.querySelector('.markdown-interactive')"), 'Partial stream was rendered'
    js("jsonProbe.classList.remove('streaming-message')")
    wait_for(lambda: js("return !!jsonProbe.querySelector('.markdown-interactive')"), 'Completed stream was not rendered')
    js("jsonProbe.remove()")
    for source in ['{unfinished', '{"root":"a","elements":{"a":{"type":"Stack","props":{},"children":["a"]}}}']:
        js("window.badJson=document.createElement('div');badJson.className='message';badJson.innerHTML='<div class=code-block><div class=code-actions></div><pre><code class=language-json-render></code></pre></div>';badJson.querySelector('code').textContent=" + json.dumps(source) + ";document.querySelector('.results-panel').append(badJson)")
        wait_for(lambda: js("return !!badJson.querySelector('.markdown-render-error')"), 'Invalid JSON did not retain source/error')
        js("badJson.querySelector('.markdown-render-error').click()")
        wait_for(lambda: js("return badJson.querySelectorAll('.markdown-render-error').length===1"), 'Retry duplicated error')
        assert js("return !badJson.querySelector('pre').hidden && !badJson.querySelector('.markdown-interactive')"), 'Invalid JSON replaced source'
        js("badJson.remove()")
    command('/refresh', {})
    wait_for(ready, 'JSON render lost after history reload')
    print('PASS: json-render chart, table/filter, keyboard tabs, source, stream, malformed/cyclic input and history reload', flush=True)
