#!/usr/bin/env python3
"""Native navigation and retained-section regression for enabled packages."""
import popovers_webkit as harness
harness.PAGE = '''<!doctype html><html data-animations="off"><head><meta charset="utf-8">
<link rel="stylesheet" href="/css/tokens.css"><link rel="stylesheet" href="/css/settings.css">
<link rel="stylesheet" href="/css/extensions.css"><style>body{margin:0}#settings{height:700px}</style>
</head><body><div data-client-view="settings"><div id="settings"></div></div>
<script type="module">
import {mountSettings} from '/ui/modules/settings-host.js';
import {mountExtensionSettings} from '/extensions/settings.js';
const frame=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
try {
 const manifest={name:'Работает',description:'Пакет',views:[{surfaces:['compact','workspace'],requires:[],entry:new URL('/unused.js',location.href).href,layout:'scroll',isolation:'shadow'}]};
 const records=[{id:'extensions',source:'builtin',settingsGroup:'builtin',required:true,enabled:true,manifest:{name:'Расширения',views:[{surfaces:['settings'],requires:['client.modules'],entry:new URL('/ui/modules/manager.js',location.href).href,layout:'form',isolation:'light'}]}},
 {id:'active',source:'package',enabled:true,manifest},{id:'off',source:'package',enabled:false,manifest:{...manifest,name:'Выключен'}},{id:'broken',source:'package',enabled:true,error:'Манифест не загружен'}];
 const values=new Map(),listeners=new Set(),storage={getItem:key=>values.get(key)??null,setItem:(key,value)=>values.set(key,value)};
 const registry={storage,state:()=>({records,bundled:[],ready:true,busy:false,notice:''}),start:()=>Promise.resolve(),subscribe(fn){listeners.add(fn);fn();return()=>listeners.delete(fn);},update(id,change){Object.assign(records.find(record=>record.id===id),change);for(const fn of listeners)fn();}};
 const services={'client.modules':signal=>({mount:root=>mountExtensionSettings(root,registry)})};
 const stop=mountSettings(document.querySelector('#settings'),registry,services,'extensions');
 while(!document.querySelector('.extension-management'))await frame();
 const checks=[],check=(ok,name)=>{if(!ok)throw Error(name);checks.push({name});};
 const nav=document.querySelector('.settings-nav'),manager=document.querySelector('.extension-management');
 check(manager.closest('.settings-content')&&!nav.querySelector('input,select,.extension-list'),'management list stays on its own page');
 const button=nav.querySelector('[data-settings-section=active]');
 check(button?.parentElement===nav&&!nav.querySelector('[data-settings-section=off],[data-settings-section=broken]'),'only enabled loaded packages become ordinary navigation entries');
 button.click();await frame();const section=document.querySelector('[data-module-page=active]'),select=section.querySelector('select');select.value='header';
 nav.querySelector('[data-settings-section=extensions]').click();await frame();
 check(section.hidden&&section.getBoundingClientRect().width===0&&select.isConnected,'switching to management hides the whole parameters section');
 button.click();await frame();const retained=section.querySelector('select')===select&&select.value==='header';registry.update('active',{enabled:false});await frame();
 check(retained&&!select.isConnected&&!nav.querySelector('[data-settings-section=active]'),'draft survives navigation; disabling removes page and runtime');
 stop();window.probe=()=>checks;
}catch(error){window.probe=()=>{throw error;};}
</script></body></html>'''
if __name__ == '__main__':
    harness.main(label='WebKitGTK ordinary extension settings pages')
