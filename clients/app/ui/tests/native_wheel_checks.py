"""Observe trusted wheel input and frame-by-frame movement in WebKit/Wayland."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time


SETUP = r"""(() => {
  const root=document.querySelector('.results-panel');
  window.nativeWheelActive=true;
  window.wheelSamples=[];window.wheelEvents=[];window.wheelAdjustments=[];
  window.wheelPointer=null;window.wheelGlobalEvents=[];
  window.addEventListener('pointermove',event=>window.wheelPointer={x:event.clientX,y:event.clientY,target:event.target.tagName,inRoot:root.contains(event.target)});
  window.addEventListener('wheel',event=>{
    if(event.isTrusted)wheelGlobalEvents.push({delta:event.deltaY,inRoot:root.contains(event.target)});
  },{capture:true,passive:true});
  root.addEventListener('wheel',event=>{
    if(event.isTrusted)wheelEvents.push({time:performance.now(),delta:event.deltaY,mode:event.deltaMode});
  },{passive:true});
  root.addEventListener('proteus-scroll-adjust',()=>wheelAdjustments.push(performance.now()));
  let previous;
  function sample(time){
    if(previous!==undefined)wheelSamples.push({time,dt:time-previous,top:root.scrollTop});
    previous=time;
    window.wheelSampleFrame=requestAnimationFrame(sample);
  }
  window.wheelSampleFrame=requestAnimationFrame(sample);
  return true;
})()"""


class NativeWheelProbe:
    def __init__(self, GLib, evaluate, present, title, done, fail):
        self.GLib, self.evaluate, self.present = GLib, evaluate, present
        self.done, self.fail = done, fail
        self.folder = tempfile.TemporaryDirectory(prefix='proteus-wheel-input-')
        socket = str(Path(self.folder.name) / 'input.sock')
        self.daemon = subprocess.Popen(['ydotoold', '--socket-path=' + socket], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.env = dict(os.environ, YDOTOOL_SOCKET=socket)
        self.title = title
        self.results = []
        self.variants = ['native', 'old-mask']
        self.index = 0
        self.stopped = False
        self.evaluate(SETUP, lambda _: self.GLib.timeout_add(2000, self.prepare))

    def prepare(self):
        if self.stopped:
            return False
        entries = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'windows']))
        native = next(item for item in entries if item.get('title') == self.title)
        subprocess.run(['niri', 'msg', 'action', 'focus-window', '--id', str(native['id'])], check=True, capture_output=True)
        if not native.get('is_fullscreen', False) and self.index == 0:
            subprocess.run(['niri', 'msg', 'action', 'fullscreen-window', '--id', str(native['id'])], check=True, capture_output=True)
        variant = self.variants[self.index]
        self.evaluate(r"""(() => {
          const root=document.querySelector('.results-panel');
          root.style.maskImage=VARIANT==='old-mask'
            ?'linear-gradient(to bottom,#000 calc(100% - var(--composer-inset,0px)),transparent calc(100% - var(--composer-inset,0px) + 22px))':'';
          root.classList.remove('sticky-bottom');
          root.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true}));
          root.scrollTop=Math.max(0,root.scrollHeight-root.clientHeight-10000);
          window.wheelReady=false;
          (async()=>{
            for(let i=0;i<16;i++)await new Promise(resolve=>requestAnimationFrame(resolve));
            wheelSamples.length=0;wheelEvents.length=0;wheelAdjustments.length=0;
            wheelGlobalEvents.length=0;
            window.wheelReady=true;
          })().catch(error=>window.wheelError=String(error));
          return true;
        })()""".replace('VARIANT', json.dumps(variant)), lambda _: self.GLib.timeout_add(100, self.wait_ready))
        return False

    def wait_ready(self):
        if self.stopped:
            return False
        def ready(value):
            if value.get('error'):
                self.fail(value['error'])
            elif value.get('ready'):
                output = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'focused-output']))
                logical = output['logical']
                subprocess.run(['ydotool', 'mousemove', '-a', '-x', str(logical['x'] + logical['width']//2),
                                '-y', str(logical['y'] + logical['height']//2)], env=self.env, check=True, capture_output=True)
                self.position_attempt = 0
                self.GLib.timeout_add(150, self.position)
            else:
                self.GLib.timeout_add(100, self.wait_ready)
        self.evaluate('({ready:window.wheelReady,error:window.wheelError})', ready)
        return False

    def position(self):
        def placed(value):
            pointer = value['pointer']
            if pointer and pointer['inRoot'] and abs(pointer['x']-value['x']) < 150 and abs(pointer['y']-value['y']) < 150:
                threading.Thread(target=self.input, daemon=True).start()
                return
            self.position_attempt += 1
            if not pointer or self.position_attempt > 10:
                self.fail('Could not place physical pointer over the transcript', value)
                return
            # Relative devices are accelerated by the compositor. Converge from
            # observed browser coordinates instead of assuming absolute pixels.
            dx, dy = round((value['x']-pointer['x'])/2), round((value['y']-pointer['y'])/2)
            subprocess.run(['ydotool', 'mousemove', '-x', str(dx), '-y', str(dy)], env=self.env, check=True, capture_output=True)
            self.GLib.timeout_add(150, self.position)
        self.evaluate('''(() => {const r=document.querySelector('.results-panel').getBoundingClientRect();
            return {pointer:wheelPointer,x:r.left+r.width/2,y:r.top+r.height*.45};})()''', placed)
        return False

    def input(self):
        try:
            time.sleep(.3)
            for _ in range(16):
                if self.stopped:
                    return
                focused = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'focused-window']))
                if focused.get('title') != self.title:
                    raise AssertionError('Test window lost focus during physical input')
                subprocess.run(['ydotool', 'mousemove', '--wheel', '-y', '1', '-x', '0'], env=self.env, check=True, capture_output=True)
                time.sleep(.14)
            time.sleep(1.5)
            self.GLib.idle_add(lambda: self.evaluate('({samples:wheelSamples,events:wheelEvents,adjustments:wheelAdjustments,pointer:wheelPointer,globalEvents:wheelGlobalEvents,root:document.querySelector(".results-panel")?.className})', self.collect) or False)
        except Exception as error:
            self.GLib.idle_add(lambda message=str(error): self.fail('Native wheel input failed: ' + message) or False)

    def collect(self, value):
        events, samples = value['events'], value['samples']
        if len(events) < 12:
            self.fail('Trusted wheel events did not reach the fixture', {key:value[key] for key in ['events','globalEvents','pointer','root']})
            return
        steps = [abs(b['top'] - a['top']) for a, b in zip(samples, samples[1:])]
        moving = [step for step in steps if step > .1]
        intervals = sorted(sample['dt'] for sample in samples)
        result = dict(variant=self.variants[self.index], events=len(events), movingFrames=len(moving),
                      distance=abs(samples[-1]['top']-samples[0]['top']), maxStep=max(steps, default=0),
                      p95FrameMs=intervals[int(len(intervals)*.95)], maxFrameMs=max(intervals),
                      adjustments=len(value['adjustments']))
        if result['distance'] < 1000 or result['movingFrames'] < 10:
            self.fail('Native wheel did not move through a meaningful transcript range', result)
            return
        self.results.append(result)
        print('Native wheel: ' + json.dumps(result), flush=True)
        self.index += 1
        if self.index < len(self.variants):
            self.prepare()
        else:
            self.done(self.results)

    def stop(self):
        self.stopped = True
        self.daemon.terminate()
        self.daemon.wait(timeout=5)
        self.folder.cleanup()
