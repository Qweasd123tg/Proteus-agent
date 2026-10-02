"""Actual markdown response, local renderer assets and SPA/stream lifecycle."""
import base64
from composer_scroll_checks import run as check_scroll
from interactive_checks import run as check_interactive, FIXTURE as INTERACTIVE_FIXTURE
from pathlib import Path

FIXTURE = r'''

## Проверка Markdown

```python
# Подсветка
print("hello", 42)
```

В строке: $E = mc^2$

$$
\sum_{k=1}^{n} k = \frac{n(n+1)}{2}
$$

<details><summary>Показать подробности</summary>

Спрятанная **подробность**.

</details>

```mermaid
flowchart LR
 A[Идея] --> B[Код]
 B --> C[Тесты]
 C --> D[Готово]
```

`$literal$` остаётся кодом.
'''
FIXTURE += INTERACTIVE_FIXTURE


def run(command, js, wait_for):
    def ready():
        return js("return document.querySelectorAll('.role-assistant mjx-container').length===2 && !!document.querySelector('.markdown-diagram svg') && !!document.querySelector('code.language-python .hljs-string')")
    wait_for(ready, 'Markdown math / syntax / Mermaid did not render')
    assert js("return [...document.querySelectorAll('.role-assistant .message code')].some(c=>c.textContent==='$literal$')"), 'Inline code became math'
    js("document.querySelector('.message summary').click()")
    assert js("return document.querySelector('.message details').open && !!document.querySelector('.message details strong')"), 'Disclosure did not open / render Markdown'
    js("document.querySelector('.code-source').click()")
    assert js("return !document.querySelector('code.language-mermaid').parentElement.hidden && document.querySelector('code.language-mermaid').textContent.includes('flowchart LR')"), 'Diagram lost its source'
    js("document.querySelector('.code-source').click()")
    assert js("return performance.getEntriesByType('resource').filter(r=>/mathjax|mermaid|highlight|woff/.test(r.name)).every(r=>new URL(r.name).origin===location.origin)"), 'Renderer used a remote asset'
    js("document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('.settings-back')"), 'Settings missing')
    js("document.querySelector('.settings-back').click()")
    wait_for(ready, 'SPA return lost Markdown renderers')
    command('/refresh', {})
    wait_for(ready, 'History reload lost Markdown renderers')
    # New content arriving while an earlier async rendering batch is running.
    js(r'''window.markdownProbe=document.createElement('div');markdownProbe.className='message streaming-message';document.querySelector('.results-panel').append(markdownProbe);markdownProbe.innerHTML='<span class="mathjax-inline">\\(a+b\\)</span>';''')
    assert js("return !markdownProbe.querySelector('mjx-container')"), 'Streaming formula was typeset too early'
    js("markdownProbe.classList.remove('streaming-message')")
    wait_for(lambda: js("return !!markdownProbe.querySelector('mjx-container')"), 'Completed streaming formula was not typeset')
    js("markdownProbe.remove()")
    js("window.invalidDiagram=document.createElement('div');invalidDiagram.className='message';invalidDiagram.innerHTML='<div class=code-block><div class=code-actions></div><pre><code class=language-mermaid>not a diagram !!!</code></pre></div>';document.querySelector('.results-panel').append(invalidDiagram)")
    wait_for(lambda: js("return !!invalidDiagram.querySelector('.markdown-render-error')"), 'Invalid diagram failed silently')
    assert js("return !invalidDiagram.querySelector('pre').hidden && invalidDiagram.querySelector('code').textContent==='not a diagram !!!' && !invalidDiagram.querySelector('svg')"), 'Invalid diagram lost its source'
    js("invalidDiagram.remove()")
    check_interactive(command, js, wait_for)
    wait_for(ready, 'Math / Markdown did not finish after interactive history reload')
    check_scroll(command, js, wait_for)
    Path('/tmp/proteus-markdown-fixed.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    print('PASS: local math, code highlight, Mermaid/source, details; SPA, reload and completed stream', flush=True)
