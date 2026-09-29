"""Measure reading-position jumps during virtual transcript window turnover."""
import json
from urllib.parse import urlencode


PROBE = r"""
      (async()=>{
        const root=document.querySelector('.results-panel');
        window.scrollJitterProgress={step:-1,phase:'warmup'};
        const frame=()=>new Promise(resolve=>requestAnimationFrame(resolve));
        const frames=async n=>{for(let i=0;i<n;i++)await frame()};
        await frames(12);
        root.classList.remove('sticky-bottom');
        root.dispatchEvent(new WheelEvent('wheel',{deltaY:-48,bubbles:true}));
        root.scrollTop=Math.max(0,root.scrollHeight-root.clientHeight-1200);
        await frames(12);
        let prototype=root, descriptor;
        while(prototype && !descriptor){
          descriptor=Object.getOwnPropertyDescriptor(prototype,'scrollTop');
          prototype=Object.getPrototypeOf(prototype);
        }
        if(!descriptor?.get || !descriptor?.set)throw Error('scrollTop accessor unavailable');
        const metrics={steps:0,samples:0,maxDrift:0,geometryWrites:0,
          noOpWrites:0,noOpSamples:[],equalAdjustEvents:0,turnovers:0,worst:null};
        let userWrite=false,lastWrite=null;
        Object.defineProperty(root,'scrollTop',{
          configurable:true,get(){return descriptor.get.call(this)},
          set(value){
            const before=descriptor.get.call(this);
            descriptor.set.call(this,value);
            if(!userWrite){
              lastWrite={before,after:descriptor.get.call(this)};
              metrics.geometryWrites++;
              if(lastWrite.after===before){
                metrics.noOpWrites++;
                if(metrics.noOpSamples.length<5)metrics.noOpSamples.push({before,requested:value,after:lastWrite.after});
              }
            }
          }
        });
        const adjusted=event=>{
          if(lastWrite && event.detail===lastWrite.before)metrics.equalAdjustEvents++;
          lastWrite=null;
        };
        root.addEventListener('proteus-scroll-adjust',adjusted);
        const windowKey=()=>[...root.querySelectorAll(':scope > [data-transcript-row]')]
          .map(row=>row.dataset.transcriptRow).join(',');
        try{
          for(let step=0;step<80;step++){
            window.scrollJitterProgress={step,phase:'scrolling',metrics};
            const bounds=root.getBoundingClientRect(),center=bounds.top+root.clientHeight/2;
            const rows=[...root.querySelectorAll(':scope > [data-transcript-row]')];
            const anchor=rows.find(row=>{
              const rect=row.getBoundingClientRect();return rect.top<=center && rect.bottom>center;
            }) || rows.reduce((best,row)=>Math.abs(row.getBoundingClientRect().top-center)<
              Math.abs(best.getBoundingClientRect().top-center)?row:best);
            if(!anchor)throw Error('No visible reading anchor');
            const id=anchor.dataset.transcriptRow,top=anchor.getBoundingClientRect().top;
            const before=root.scrollTop,key=windowKey(),delta=step<40?-48:48;
            root.dispatchEvent(new WheelEvent('wheel',{deltaY:delta,bubbles:true}));
            userWrite=true;root.scrollTop=before+delta;userWrite=false;
            const intended=root.scrollTop-before;
            if(Math.abs(intended-delta)>1)throw Error('Probe reached transcript boundary');
            for(let sample=0;sample<4;sample++){
              await frame();
              const current=[...root.querySelectorAll(':scope > [data-transcript-row]')]
                .find(row=>row.dataset.transcriptRow===id);
              if(!current)throw Error('Visible reading anchor was evicted');
              const drift=Math.abs(current.getBoundingClientRect().top-top+intended);
              metrics.samples++;
              if(drift>metrics.maxDrift){metrics.maxDrift=drift;metrics.worst={step,sample,id,drift}}
            }
            if(key!==windowKey())metrics.turnovers++;
            metrics.steps++;
          }
          return metrics;
        }finally{
          root.removeEventListener('proteus-scroll-adjust',adjusted);
          delete root.scrollTop;
        }
      })()
"""


def validate(result):
    print('Scroll jitter: ' + json.dumps(result, ensure_ascii=False), flush=True)
    assert 'error' not in result, result
    assert result['steps'] == 80 and result['turnovers'] > 0, result
    assert result['maxDrift'] <= 1, 'Reading anchor jumped: ' + str(result)
    assert result['noOpWrites'] == 0 and result['equalAdjustEvents'] == 0, result
    return result


def run(command, js, wait_for, web, origin):
    command('/url', {'url': web + '/foundation.html?' + urlencode({
        'server': origin, 'token': 'extension-smoke'})})
    wait_for(lambda: js("""
        const root=document.querySelector('.results-panel');
        return !!window.fixtureLastSnapshot && root?.clientHeight>0 &&
          +root.dataset.transcriptCount>=240 && root.querySelector('[data-transcript-row]') &&
          root.scrollHeight-root.clientHeight-root.scrollTop<2;
    """), 'Long authoritative transcript did not render its tail')
    result = command('/execute/async', {'args': [], 'script':
        'const done=arguments[arguments.length-1];\n' + PROBE +
        '.then(done,error=>done({error:String(error)}));'})
    return validate(result)
