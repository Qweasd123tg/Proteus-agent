#!/usr/bin/env python3
"""Native engine regression for settings controls and hover-stable tabs."""
import popovers_webkit as harness
harness.PAGE = '''<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css"><link rel="stylesheet" href="/css/settings.css"><link rel="stylesheet" href="/ui/controls.css">
<link rel="stylesheet" href="/css/extension-columns.css"></head><body>
<div class="settings-content" style="width:900px;padding:0" id="wide"></div>
<div class="settings-content" style="width:320px;padding:0" id="narrow"></div>
<div class="workspace-group focused"><div class="workspace-tabs"><div class="workspace-tab active"><button class="workspace-tab-name">Один</button><button class="workspace-tab-close">×</button></div><div class="workspace-tab"><button class="workspace-tab-name">Два</button><button class="workspace-tab-close">×</button></div></div></div>
<script type="module">
import {mount as chat} from '/ui/modules/chat.js';
import {mount as appearance} from '/ui/modules/appearance.js';
const state={sendMode:'enter',toolCardsCollapsed:true,autoScroll:true,fontSize:16,chatWidth:1600,animations:true};
const controller=new AbortController();const services={'client.preferences':{read:()=>state,set:(k,v)=>state[k]=v,subscribe:()=>()=>{}}};
for(const id of ['wide','narrow']){const root=document.getElementById(id);chat({root,services,signal:controller.signal});appearance({root,services,signal:controller.signal});}
window.probe=()=>{
 window.probe=null;
 const checks=[];const check=(ok,name)=>{if(!ok)throw Error(name);checks.push({name});};
 const wide=document.querySelector('#wide select'),label=wide.previousElementSibling;
 check(label.getBoundingClientRect().width>=400 && wide.getBoundingClientRect().width<=210,'wide label retains width');
 const narrow=document.querySelector('#narrow select');check(narrow.getBoundingClientRect().top>=narrow.previousElementSibling.getBoundingClientRect().bottom && narrow.getBoundingClientRect().right<=320,'narrow select wraps');
 const range=document.querySelector('[aria-label="Ширина диалога"]'),style=getComputedStyle(range),thumb=getComputedStyle(range,'::-webkit-slider-thumb');
 check(style.paddingLeft==='0px'&&style.paddingRight==='0px'&&range.value==='1600'&&style.appearance==='none','range has no inherited textbox inset');
 const tab=document.querySelector('.workspace-tab:not(.active)'),close=tab.querySelector('.workspace-tab-close'),before=tab.getBoundingClientRect().width,resting=getComputedStyle(close);
 const visible=resting.opacity==='1'&&resting.pointerEvents!=='none';close.focus();
 check(visible&&close.getBoundingClientRect().width===26&&tab.getBoundingClientRect().width===before&&getComputedStyle(document.querySelector('.workspace-tab.active')).boxShadow==='none','close remains visible without changing width; active tab has no underline');
 return checks;
};</script></body></html>'''
if __name__ == '__main__':
    harness.main(label='WebKitGTK settings controls and stable tabs')
