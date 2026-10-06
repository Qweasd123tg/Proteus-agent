"""Rendered motion, interruption and retained state through real client controls."""
import json


def run(command, js, wait_for):
    reduced = js("return matchMedia('(prefers-reduced-motion: reduce)').matches")
    js(r"""
      window.motionFrames = count => new Promise(resolve => {
        function frame() { if (--count <= 0) resolve(); else requestAnimationFrame(frame); }
        requestAnimationFrame(frame);
      });
      window.motionRead = root => {
        const style = getComputedStyle(root), rect = root.getBoundingClientRect();
        return {opacity:+style.opacity, height:rect.height, width:rect.width,
          hidden:root.hidden, inert:root.inert, display:style.display,
          running:root.getAnimations().some(a=>a.playState==='running'),
          interactive:style.pointerEvents!=='none'&&!root.inert,
          popover:root.hasAttribute('popover')&&root.matches(':popover-open')};
      };
      window.motionSample = async (root, action, middle, interrupt) => {
        const samples=[], started=performance.now(); let interrupted=false, afterInterrupt=null, idle=0, previous="";
        action();
        while(performance.now()-started<3000) {
          await motionFrames(1);
          const frame=motionRead(root); samples.push(frame);
          const rendered=JSON.stringify([frame.opacity,frame.height,frame.width,frame.display]);
          if(interrupt&&!interrupted&&middle(frame,samples.length)) {
            interrupted=true; interrupt();
            afterInterrupt=motionFrames(2).then(()=>motionRead(root));
            idle=0;
          } else idle=frame.running||rendered!==previous?0:idle+1;
          previous=rendered;
          if(samples.length>=5&&idle>=3&&(!interrupt||interrupted))
            return {samples,interrupted,afterInterrupt:await afterInterrupt};
        }
        return {samples,interrupted,afterInterrupt:await afterInterrupt,timeout:true};
      };
    """)

    def click(selector):
        js('document.querySelector('+json.dumps(selector)+').click()')

    def frames():
        command('/execute/async', {'script': 'const done=arguments[arguments.length-1];motionFrames(3).then(()=>done(null))', 'args': []})

    def sample(selector, action, middle='v=>v.opacity>0&&v.opacity<1', interrupt=None):
        script = 'const done=arguments[arguments.length-1];motionSample(document.querySelector('+json.dumps(selector)+'),()=>{'+action+'},'+middle+','+('()=>{'+interrupt+'}' if interrupt else 'null')+').then(done,error=>done({error:String(error)}))'
        result = command('/execute/async', {'script': script, 'args': []})
        diagnostic = {**result, 'samples':result.get('samples',[])[:6]+result.get('samples',[])[-3:]}
        assert not result.get('error') and not result.get('timeout'), 'Motion did not settle: '+str(diagnostic)
        return result

    def opacity_motion(result, label):
        intermediate = any(0 < s['opacity'] < 1 for s in result['samples'])
        assert intermediate != reduced, label + (' animated despite reduced motion' if reduced else ' jumped without a visible transition')

    def click_action(selector):
        return 'document.querySelector('+json.dumps(selector)+').click()'

    settings = '[data-client-view=settings]'
    board = '[data-client-workspace]'
    wait_for(lambda: js("return !!document.querySelector('.composer textarea') && !!document.querySelector('.composer-model-menu')"), 'Motion fixture not ready')
    js("window.motionChat=document.querySelector('.session-workspace');window.motionComposer=document.querySelector('.composer textarea');motionComposer.value='Черновик при смене экранов';motionComposer.dispatchEvent(new Event('input',{bubbles:true}));window.motionLayout=localStorage.getItem('proteus.workspace.layout')")
    # Prewarm lazy settings controls; entering a screen must preserve its owner.
    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-animation-toggle]')"), 'Appearance controls not mounted')
    if not js("return document.querySelector('[data-animation-toggle]').checked"):
        click('[data-animation-toggle]')
    click('.settings-back')
    frames()
    # Screens swap at once: a crossfade overlaps two pages and flickers.
    enter = sample(settings, click_action('.settings-link'))
    assert not any(0 < v['opacity'] < 1 or v['running'] for v in enter['samples']), 'Settings entry animated instead of swapping'
    assert js("return !document.querySelector('[data-client-view=settings]').hidden && document.querySelector('[data-client-workspace]').hidden && document.querySelector('[data-client-workspace]').inert"), 'Settings entry left the previous screen interactive'
    leave = sample(settings, click_action('.settings-back'))
    assert all(s['hidden'] and s['inert'] and s['display'] == 'none' for s in leave['samples']), 'Leaving screen stayed painted or interactive'
    click('.settings-link')
    frames()
    assert js("const b=document.querySelector('.settings-back'),r=b.getBoundingClientRect();return b.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))"), 'Reopened screen is obscured by an outgoing surface'
    click('.settings-back')
    frames()
    assert js("return document.querySelector('.session-workspace')===motionChat && document.querySelector('.composer textarea')===motionComposer && motionComposer.value==='Черновик при смене экранов' && localStorage.getItem('proteus.workspace.layout')===motionLayout"), 'Screen animation reset workspace, draft or retained roots'

    # The installation disclosure must grow and collapse, including reversal.
    click('.settings-link');click('[data-settings-section=extensions]')
    wait_for(lambda: js("return !!document.querySelector('.extension-source')"), 'Extension source disclosure missing')
    details = '.extension-source'
    summary = '.extension-source > summary'
    closed_height = js("return document.querySelector('.extension-source').getBoundingClientRect().height")
    opening = sample(details, click_action(summary))
    full_height = opening['samples'][-1]['height']
    assert full_height > closed_height + 50, 'Disclosure did not expose its controls'
    intermediate = any(closed_height + 1 < v['height'] < full_height - 1 for v in opening['samples'])
    assert intermediate != reduced, 'Disclosure opening ignores the motion preference or jumps to full height: '+str({'closed':closed_height,'full':full_height,'samples':opening['samples'][:8]})
    closing = sample(details, click_action(summary))
    intermediate = any(closed_height + 1 < v['height'] < full_height - 1 for v in closing['samples'])
    assert intermediate != reduced, 'Disclosure closing ignores the motion preference or collapses abruptly'
    sample(details, click_action(summary))
    reversal = sample(details, click_action(summary),
                      middle='(v,n)=>n===2' if reduced else 'v=>v.height>'+str(closed_height+1)+'&&v.height<'+str(full_height-1),
                      interrupt=click_action(summary))
    assert reversal['interrupted'] and js("return document.querySelector('.extension-source').open"), 'Disclosure reversal ended closed'
    assert abs(reversal['samples'][-1]['height']-full_height)<2, 'Interrupted disclosure clipped its controls'
    click('.settings-back');frames()

    # Native popover closure ends interaction immediately while its pixels may fade.
    picker = '.workspace-picker'
    opening = sample(picker, click_action('.workspace-add'))
    opacity_motion(opening, 'Tab picker entry')
    escape = "document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',code:'Escape',bubbles:true,cancelable:true}))"
    closing = sample(picker, escape)
    opacity_motion(closing, 'Tab picker exit')
    assert all(not v['popover'] and not v['interactive'] for v in closing['samples']), 'Closing menu continued accepting input'
    sample(picker, click_action('.workspace-add'))
    reopened = sample(picker, escape,
                      middle='(v,n)=>n===2' if reduced else 'v=>v.running&&v.opacity>0&&v.opacity<1',
                      interrupt=click_action('.workspace-add'))
    assert reopened['interrupted'] and js("return document.querySelector('.workspace-picker').matches(':popover-open') && !document.querySelector('.workspace-picker').inert"), 'An old close dismissed the reopened menu'
    click('.workspace-picker [data-open-tab=files]')
    wait_for(lambda: js("return document.querySelector('[data-tab-id=files]')?.classList.contains('active')"), 'Reopened menu cannot select a tab')
    click('.brand')

    # The motion preference still applies live from its setting.
    click('.settings-link');click('[data-settings-section=appearance]')
    click('[data-animation-toggle]')
    assert js("return document.documentElement.dataset.animations==='off'"), 'Motion preference was not applied'
    click('[data-animation-toggle]');click('.settings-back')
    print('PASS: instant screen swap, interrupted disclosure/popover reopening, immediate inertness, retained draft/layout, live motion preference; reduced='+str(reduced),flush=True)
