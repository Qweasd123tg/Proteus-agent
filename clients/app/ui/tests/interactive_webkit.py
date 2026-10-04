#!/usr/bin/env python3
"""Native engine check for lazy JSON rendering and composer scroll geometry."""
import json
from interactive_checks import SOURCE
import popovers_webkit as harness

harness.PAGE = '''<!doctype html><html data-animations="off"><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css"><link rel="stylesheet" href="/css/chat.css">
<link rel="stylesheet" href="/css/composer.css"><link rel="stylesheet" href="/css/layout.css">
<link rel="stylesheet" href="/css/markdown.css"><link rel="stylesheet" href="/css/interactive.css">
</head><body><div class="session-workspace" style="height:900px;--chat-max-width:820px">
<section class="results-panel sticky-bottom"><div class="message"><div class="code-block"><div class="code-actions"></div><pre><code class="language-json-render"></code></pre></div></div>
<div style="height:1000px"></div><p id="last">Последняя строка</p></section>
<form class="composer"><div class="composer-shell" style="height:140px">Поле ввода</div></form></div>
<script type="module">
import {mountComposerDock} from '/ui/layout.js';
const code=document.querySelector('code');code.textContent=SOURCE;
mountComposerDock(document.querySelector('form'));
await import('/ui/markdown.js');
window.probe=()=>{
 if(!document.querySelector('.markdown-interactive'))return null;
 const results=[];
 function check(ok,name){if(!ok)throw Error(name);results.push({name});}
 check(document.querySelectorAll('.jr-bar').length===2,'native chart');
 document.querySelectorAll('.jr-tab')[1].click();
 const search=document.querySelector('.jr-search');search.value='После';search.dispatchEvent(new Event('input'));
 check(document.querySelectorAll('tbody tr').length===1 && document.querySelector('tbody').textContent.includes('35'),'native tabs/filter');
 document.querySelector('.code-source').click();
 check(!code.parentElement.hidden&&code.textContent===SOURCE,'native source toggle');
 const workspace=document.querySelector('.session-workspace'),r=document.querySelector('section');
 workspace.style.setProperty('--chat-max-width','780px');r.scrollTop=r.scrollHeight;
 check(document.querySelector('#last').getBoundingClientRect().bottom<=document.querySelector('form').getBoundingClientRect().top-16 && getComputedStyle(r).maskImage==='none' && getComputedStyle(document.querySelector('form'),'::before').backgroundImage.includes('gradient'),'native composer clearance/fade');
 return results;
};
</script></body></html>'''.replace('SOURCE', json.dumps(SOURCE))

if __name__ == '__main__':
    harness.main(label='WebKitGTK json-render and composer geometry')
