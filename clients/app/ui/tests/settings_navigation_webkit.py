#!/usr/bin/env python3
"""Native visibility regression for extension settings outside their section."""
import popovers_webkit as harness
harness.PAGE = '''<!doctype html><html data-animations="off"><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css"><link rel="stylesheet" href="/css/settings.css">
<link rel="stylesheet" href="/css/extensions.css"><link rel="stylesheet" href="/css/extension-settings.css">
<style>body{margin:0}.settings-page{height:700px}.fixture-row{height:70px}</style>
</head><body><div data-client-view="settings"><div class="settings-page"><nav class="settings-nav"><button>Внешний вид</button><section class="settings-extensions-nav"><button class="settings-extensions-heading settings-nav-label">Расширения</button></section></nav>
<main class="settings-content"><section class="settings-section" data-module-page="extensions"><div class="extension-settings"></div></section></main></div></div>
<script type="module">
import {createSettingsLayout} from '/extensions/settings-layout.js';
import {createSettingsPane} from '/extensions/settings-pane.js';
const frame=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
try {
 const root=document.querySelector('.extension-settings'),section=root.parentElement,nav=document.querySelector('.settings-nav'),view=document.querySelector('[data-client-view]'),controller=new AbortController();
 const layout=createSettingsLayout(root),pane=createSettingsPane(root,controller.signal,()=>{});
 for(let i=0;i<25;i++){const row=document.createElement('div');row.className='fixture-row';row.textContent='Расширение '+i;layout.sidebar.append(row);}
 const input=document.createElement('textarea');input.value='Черновик';pane.body.append(input);pane.show('Параметры');
 document.addEventListener('proteus-select-settings-module',event=>{if(event.detail==='extensions')section.hidden=false;},{signal:controller.signal});
 await frame();const checks=[];const check=(ok,name)=>{if(!ok)throw Error(name);checks.push({name});};
 const n=nav.getBoundingClientRect(),s=layout.sidebar.getBoundingClientRect(),p=pane.element.getBoundingClientRect();
 check(s.left>=n.left&&s.right<=n.right&&Math.abs(n.right-p.left)<=1&&getComputedStyle(nav.parentElement).gridTemplateColumns.split(' ').length===2,'extension group stays inside navigation; only two columns');
 nav.scrollTop=220;await frame();check(nav.scrollTop===220&&document.querySelector('.settings-content').scrollTop===0,'navigation scroll is independent of parameters');
 section.hidden=true;await frame();check(!layout.sidebar.hidden&&!layout.sidebar.inert&&layout.sidebar.getBoundingClientRect().width>0&&pane.element.getBoundingClientRect().width===0,'group remains available while another section hides parameters');
 layout.activate();await frame();view.hidden=true;await frame();view.hidden=false;await frame();check(!section.hidden&&nav.scrollTop===220&&input.value==='Черновик'&&input.isConnected&&pane.element.getBoundingClientRect().width>0,'selecting an extension restores parameters; scroll and draft survive hiding settings');
 controller.abort();pane.remove();layout.remove();window.probe=()=>checks;
} catch(error){window.probe=()=>{throw error;};}
</script></body></html>'''
if __name__ == '__main__':
    harness.main(label='WebKitGTK extensions inside settings navigation')
