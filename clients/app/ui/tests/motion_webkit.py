#!/usr/bin/env python3
"""Native retained-view and settings-disclosure animation lifecycle checks."""
import popovers_webkit as harness

harness.PAGE = '''<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css">
<link rel="stylesheet" href="/css/motion.css">
<style>
.screen { display:flex; width:480px; height:160px; padding:20px; background:#333; }
.screen[hidden] { display:none; }
details { width:480px; border:1px solid #666; padding:6px; }
summary { height:34px; }
.disclosure-content { height:120px; padding:12px; }
</style></head><body>
<section class="screen" id="first"><button>First view</button></section>
<section class="screen" id="second" hidden inert><button>Second view</button></section>
<details open><summary>Settings</summary><div class="disclosure-content"><button>Setting</button></div></details>
<script type="module">
import {watchViewMotion} from '/ui/view-motion.js';
import {mountDisclosureMotion} from '/ui/disclosure-motion.js';
import {applyMotion} from '/ui/motion.js';
const first=document.querySelector('#first'),second=document.querySelector('#second');
const details=document.querySelector('details'),summary=details.querySelector('summary'),content=details.querySelector('.disclosure-content');
const controller=new AbortController();
watchViewMotion(first,{signal:controller.signal});watchViewMotion(second,{signal:controller.signal});
mountDisclosureMotion(details,content,controller.signal);
localStorage.setItem('proteus.animations','true');applyMotion();
const checks=[],frame=()=>new Promise(resolve=>requestAnimationFrame(resolve));
const assert=(condition,message)=>{if(!condition)throw Error(message);};
const running=element=>element.getAnimations().filter(animation=>animation.playState!=='finished');
const choose=element=>{for(const node of [first,second]){node.hidden=node!==element;node.inert=node!==element;}};
async function flush(){await Promise.resolve();await frame();}
async function middle(element,time){const animation=running(element)[0];assert(animation,'Animation missing: '+element.tagName);animation.pause();animation.currentTime=time;await frame();return animation;}
async function finish(element){for(const animation of running(element))animation.finish();await Promise.resolve();await frame();}
function cleanView(element){return !element.hasAttribute('data-view-exit')&&!element.style.getPropertyValue('--view-display')&&!running(element).length;}
(async()=>{
 await frame();await frame();const rectangle=first.getBoundingClientRect();
 choose(second);await flush();await middle(first,65);await middle(second,100);
 const outgoing=Number(getComputedStyle(first).opacity),incoming=Number(getComputedStyle(second).opacity),exitRect=first.getBoundingClientRect();
 assert(outgoing>0&&outgoing<1&&incoming>0&&incoming<1,'Enter/exit did not reach intermediate opacity');
 assert(first.hidden&&first.inert&&first.hasAttribute('data-view-exit')&&!second.hidden&&!second.inert,'Exit lost immediate hidden/inert semantics');
 assert(first.parentElement===document.body&&Math.abs(exitRect.left-rectangle.left)<1&&Math.abs(exitRect.top-rectangle.top)<1&&Math.abs(exitRect.width-rectangle.width)<1&&getComputedStyle(first).pointerEvents==='none','Exit moved DOM, changed rectangle or retained hit testing');
 checks.push({name:'intermediate enter/exit opacity; immediate inert and stable outgoing rectangle'});

 choose(first);await flush();choose(second);await flush();choose(first);await flush();
 await finish(first);await finish(second);
 assert(!first.hidden&&!first.inert&&second.hidden&&second.inert&&cleanView(first)&&cleanView(second),'Rapid reversal did not settle to latest visibility');
 assert(getComputedStyle(second).display==='none'&&getComputedStyle(first).opacity==='1','Settled views retain exit presentation');
 // A parent FLIP already supplies motion: a second viewport exit would drift.
 const moving=document.createElement('div'),movingView=document.createElement('section');
 moving.style.transform='translateX(24px)';movingView.className='screen';movingView.textContent='View inside a moving ancestor';moving.append(movingView);document.body.append(moving);
 watchViewMotion(movingView,{signal:controller.signal});await flush();movingView.hidden=true;movingView.inert=true;await flush();
 assert(movingView.hidden&&movingView.inert&&cleanView(movingView)&&getComputedStyle(movingView).display==='none','Transformed ancestor retained a drifting exit overlay');
 movingView.hidden=false;movingView.inert=false;await flush();await middle(movingView,100);
 assert(Number(getComputedStyle(movingView).opacity)>0&&Number(getComputedStyle(movingView).opacity)<1,'Transformed ancestor suppressed incoming fade');
 await finish(movingView);assert(cleanView(movingView)&&!movingView.hidden&&!movingView.inert,'Transformed ancestor did not settle');moving.remove();
 for(const boundary of ['contain:layout','container-type:inline-size']) {
  const host=document.createElement('div'),view=document.createElement('section'),before=document.createElement('div'),after=document.createElement('div');
  host.style.cssText=boundary+';width:560px;height:220px;margin-left:36px;border:7px solid #777;padding:19px;overflow:auto';
  before.style.cssText='height:40px;width:800px';after.style.cssText='height:400px;width:800px';view.className='screen';view.textContent=boundary;host.append(before,view,after);document.body.append(host);
  host.scrollTop=37;host.scrollLeft=23;await flush();await frame();
  const probe=document.createElement('div');probe.style.cssText='position:fixed;left:0;top:0;width:1px;height:1px';host.append(probe);
  const probeRect=probe.getBoundingClientRect(),hostRect=host.getBoundingClientRect();
  const formsBlock=Math.abs(probeRect.left-(hostRect.left+host.clientLeft-host.scrollLeft))<1&&Math.abs(probeRect.top-(hostRect.top+host.clientTop-host.scrollTop))<1;
  if(boundary==='contain:layout')assert(formsBlock,'Layout containment did not establish a fixed containing block');
  else window.containerTypeFormsFixedBlock=formsBlock;
  probe.remove();
  watchViewMotion(view,{signal:controller.signal});await flush();const previousRect=view.getBoundingClientRect();view.hidden=true;view.inert=true;await flush();await middle(view,65);const closing=view.getBoundingClientRect();
  assert(view.hidden&&view.inert&&view.hasAttribute('data-view-exit')&&Math.abs(closing.left-previousRect.left)<1&&Math.abs(closing.top-previousRect.top)<1&&Math.abs(closing.width-previousRect.width)<1&&Math.abs(closing.height-previousRect.height)<1,boundary+' exit drift: before '+previousRect.left+','+previousRect.top+' after '+closing.left+','+closing.top);
  await finish(view);assert(cleanView(view)&&getComputedStyle(view).display==='none',boundary+' retained exit state');host.remove();
 }
 checks.push({name:'rapid reversal; transformed ancestor skips exit; scrolled containment preserves exit rectangle',containerTypeFixedBlock:window.containerTypeFormsFixedBlock});

 const full=details.getBoundingClientRect().height;summary.click();await flush();await middle(details,80);
 const partial=details.getBoundingClientRect().height;
 assert(partial>summary.getBoundingClientRect().height+14&&partial<full,'Closing disclosure has no intermediate height');
 assert(details.open&&content.inert&&summary.getAttribute('aria-expanded')==='false','Closing disclosure remains interactive or loses summary semantics');
 summary.click();await flush();await middle(details,100);
 assert(details.getBoundingClientRect().height>partial&&details.getBoundingClientRect().height<full,'Reopening disclosure jumps instead of continuing');
 await finish(details);
 assert(details.open&&!content.inert&&summary.getAttribute('aria-expanded')==='true'&&Math.abs(details.getBoundingClientRect().height-full)<1&&details.style.height===''&&details.style.overflow==='','Disclosure did not restore full unclipped content');
 summary.click();await flush();await finish(details);
 assert(!details.open&&content.inert,'Completed disclosure close lost final state');
 checks.push({name:'disclosure close/reopen has intermediate height and complete final content'});

 choose(second);summary.click();await flush();
 localStorage.setItem('proteus.animations','false');applyMotion();await flush();
 assert(cleanView(first)&&cleanView(second)&&first.hidden&&!second.hidden&&!second.inert,'Live motion off retained a view animation');
 assert(details.open&&!content.inert&&details.style.height===''&&details.style.overflow===''&&!running(details).length,'Live motion off left a clipped disclosure');
 localStorage.setItem('proteus.animations','true');applyMotion();choose(first);summary.click();await flush();controller.abort();await flush();
 assert(cleanView(first)&&cleanView(second)&&!details.open&&content.inert&&details.style.height===''&&details.style.overflow===''&&!running(details).length,'Disposal retained temporary motion state');
 checks.push({name:'live off and disposal restore final view and disclosure state'});
 window.motionResult=checks;
})().catch(error=>window.motionFailure=error.message+'\\n'+(error.stack||''));
window.probe=()=>{if(window.motionFailure)throw Error(window.motionFailure);if(!window.motionResult)return null;window.probe=null;return window.motionResult;};
</script></body></html>'''

if __name__ == '__main__':
    harness.main(label='WebKitGTK retained views and settings disclosures')
