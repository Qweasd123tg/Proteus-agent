#!/usr/bin/env python3
"""Native settings-disclosure animation lifecycle checks."""
import popovers_webkit as harness

harness.PAGE = '''<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css">
<link rel="stylesheet" href="/css/motion.css">
<style>
details { width:480px; border:1px solid #666; padding:6px; }
summary { height:34px; }
.disclosure-content { height:120px; padding:12px; }
</style></head><body>
<details open><summary>Settings</summary><div class="disclosure-content"><button>Setting</button></div></details>
<script type="module">
import {mountDisclosureMotion} from '/ui/disclosure-motion.js';
import {applyMotion} from '/ui/motion.js';
const details=document.querySelector('details'),summary=details.querySelector('summary'),content=details.querySelector('.disclosure-content');
const controller=new AbortController();
mountDisclosureMotion(details,content,controller.signal);
localStorage.setItem('proteus.animations','true');applyMotion();
const checks=[],frame=()=>new Promise(resolve=>requestAnimationFrame(resolve));
const assert=(condition,message)=>{if(!condition)throw Error(message);};
const running=element=>element.getAnimations().filter(animation=>animation.playState!=='finished');
async function flush(){await Promise.resolve();await frame();}
async function middle(element,time){const animation=running(element)[0];assert(animation,'Animation missing: '+element.tagName);animation.pause();animation.currentTime=time;await frame();return animation;}
async function finish(element){for(const animation of running(element))animation.finish();await Promise.resolve();await frame();}
(async()=>{
 await frame();await frame();
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

 summary.click();await flush();
 localStorage.setItem('proteus.animations','false');applyMotion();await flush();
 assert(details.open&&!content.inert&&details.style.height===''&&details.style.overflow===''&&!running(details).length,'Live motion off left a clipped disclosure');
 localStorage.setItem('proteus.animations','true');applyMotion();summary.click();await flush();controller.abort();await flush();
 assert(!details.open&&content.inert&&details.style.height===''&&details.style.overflow===''&&!running(details).length,'Disposal retained temporary motion state');
 checks.push({name:'live off and disposal restore final disclosure state'});
 window.motionResult=checks;
})().catch(error=>window.motionFailure=error.message+'\\n'+(error.stack||''));
window.probe=()=>{if(window.motionFailure)throw Error(window.motionFailure);if(!window.motionResult)return null;window.probe=null;return window.motionResult;};
</script></body></html>'''

if __name__ == '__main__':
    harness.main(label='WebKitGTK settings disclosures', count=2)
