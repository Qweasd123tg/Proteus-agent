"""Shared header and workspace geometry with a long transcript."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    for menu in ['access', 'model', 'access']:
        js(f"document.querySelector('.composer-{menu}-menu summary').click()")
        wait_for(lambda: js(f"return document.querySelectorAll('.composer-menu[open]').length===1 && document.querySelector('.composer-{menu}-menu').open"), 'Composer menus overlap')
    js("document.querySelector('.composer textarea').click()")
    assert js("return !document.querySelector('.composer-menu[open]')"), 'Input did not dismiss menu'
    js("window.perfFixture=document.createElement('div');for(let i=0;i<240;i++){const p=document.createElement('article');p.className='task-card';p.textContent=('Representative transcript text with paths and code fragments. ').repeat(35);window.perfFixture.append(p)}document.querySelector('.results-panel').append(window.perfFixture)")
    for selector in ['[data-workspace-toggle]', '[data-panel-toggle=sidebar]']:
        result=command('/execute/async', {'script': '''
          const done=arguments[arguments.length-1],button=document.querySelector(arguments[0]);
          const chat=document.querySelector('.session-workspace'), widths=[];
          button.focus();button.click();let frames=0;
          function frame(){widths.push(Math.round(chat.getBoundingClientRect().width));if(++frames<12)requestAnimationFrame(frame);else done({widths:[...new Set(widths)],animated:document.getAnimations().some(a=>a.effect?.target?.matches('.workspace-main,.tab-workspace,.sidebar')&&a.effect.getKeyframes().some(k=>'width' in k||'height' in k))});}
          requestAnimationFrame(frame);
        ''', 'args':[selector]})
        assert len(result['widths'])==1 and not result['animated'], 'Toggle animates chat geometry'
        js(f"document.querySelector('{selector}').click()")
    js("window.perfFixture.remove();if(document.querySelector('.tab-workspace').hidden)document.querySelector('[data-workspace-toggle]').click();window.beforeWidth=document.querySelector('.tab-workspace').getBoundingClientRect().width;document.querySelector('.workspace-resize').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}))")
    assert js("return document.querySelector('.tab-workspace').getBoundingClientRect().width>window.beforeWidth && Number(localStorage.getItem('proteus.ui.workspace.width'))>window.beforeWidth"), 'Workspace resize did not persist'
    js("window.keptWorkspace=document.querySelector('.tab-workspace');document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('.extension-settings')"), 'Settings missing')
    js("document.querySelector('[aria-label=Назад]').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"), 'Header Back did not restore chat')
    assert js("return document.querySelector('.tab-workspace')===window.keptWorkspace"), 'Navigation remounted workspace'
    js("document.querySelector('[aria-label=Вперёд]').click()")
    wait_for(lambda: js("return !!document.querySelector('.extension-settings')"), 'Header Forward failed')
    js("document.querySelector('.return-to-chat').click()")
    wait_for(lambda: js("return !!document.querySelector('.composer textarea')"), 'Chat did not restore')
    for width in [1440, 900, 620]:
        command('/window/rect',{'width':width,'height':1000})
        assert js("return document.documentElement.scrollWidth<=Math.max(innerWidth,860)"), 'Horizontal overflow'
    command('/window/rect',{'width':1440,'height':1000})
    Path('/tmp/proteus-tab-layout.png').write_bytes(base64.b64decode(command('/screenshot',None)))
    print('PASS: immediate panel geometry; width persistence; Back/Forward; desktop shell',flush=True)
