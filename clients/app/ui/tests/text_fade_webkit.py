#!/usr/bin/env python3
"""Native WebKit check of the fade on hover-scrolled single-line labels."""
import popovers_webkit as harness

harness.PAGE = '''<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css">
<link rel="stylesheet" href="/ui/text-overflow.css">
<style>.probe-label { width:220px; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; font-size:14px; }</style>
</head><body>
<div class="probe-label">Очень длинное название чата, которое явно не помещается в отведённую ширину строки</div>
<script type="module">
import '/ui/text-overflow.js';
const frame=()=>new Promise(resolve=>requestAnimationFrame(resolve));
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const assert=(condition,message)=>{if(!condition)throw Error(message);};
const fadeStart=element=>parseFloat(getComputedStyle(element).maskImage.match(/([\\d.]+)px/)?.[1] ?? 'NaN');
(async()=>{
 const label=document.querySelector('.probe-label');
 await frame();await frame();
 const rest=getComputedStyle(label).maskImage;
 assert(label.scrollWidth>label.clientWidth+28&&fadeStart(label)===0&&rest.includes('100% - 28px'),'Overflowing label has no right-edge fade at rest: '+rest);
 label.dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
 const began=performance.now(),moved=[];
 while(performance.now()-began<3000){
  await frame();
  if(label.scrollLeft>0)moved.push({left:label.scrollLeft,start:fadeStart(label)});
 }
 assert(moved.length>5,'Hovered label did not scroll');
 const cut=moved.find(sample=>!(sample.start+0.5>=Math.min(28,sample.left)));
 assert(!cut,'Scrolled-out text is cut before the fade: '+JSON.stringify(cut));
 label.dispatchEvent(new PointerEvent('pointerout',{bubbles:true,relatedTarget:document.body}));
 await sleep(1200);
 assert(label.scrollLeft===0&&label.style.maskImage===''&&getComputedStyle(label).maskImage===rest,'Label did not return to its resting fade');
 window.fadeResult=[{name:'hover scroll keeps the fade ahead of hidden text',frames:moved.length},{name:'leaving restores the resting fade'}];
})().catch(error=>window.fadeFailure=error.message);
window.probe=()=>{if(window.fadeFailure)throw Error(window.fadeFailure);if(!window.fadeResult)return null;window.probe=null;return window.fadeResult;};
</script></body></html>'''

if __name__ == '__main__':
    harness.main(label='WebKitGTK label fade', count=2)
