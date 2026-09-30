#!/usr/bin/env python3
"""Check native WebKitGTK popup sizing; Firefox does not reproduce this regression.

Requires Python GI, GTK3, WebKit2 4.1 and Xvfb. Uses production sidebar/CSS,
no backend, accounts or changes to the user's running desktop session.
"""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
PAGE = '''<!doctype html><html data-desktop-chrome data-animations="off"><head>
<meta charset="utf-8"><link rel="stylesheet" href="/css/tokens.css">
<link rel="stylesheet" href="/ui/popup.css"><link rel="stylesheet" href="/ui/select.css"><link rel="stylesheet" href="/css/composer-menu.css"><script type="module" src="/ui/select.js"></script></head><body>
<div id="choices" style="position:fixed;left:1120px;top:900px;width:230px"><select aria-label="Long choice"><option value="a" data-description="Пояснение длинного пункта без обрезания">Название длинного пункта с параметрами модели и дополнительными условиями выбора</option><option value="b">Другой пункт</option></select><div id="model-menu"></div><div id="shadow-choice"></div></div><aside id="sidebar">
<button data-workspace="/tmp/project" style="position:absolute;left:130px;top:220px">
<span data-sidebar-menu>Проект</span></button>
<div data-session-dir="/tmp/session" data-hover-title="Чат">
<button data-sidebar-menu>Чат</button></div></aside>
<script type="module">
import {mountSidebar} from '/ui/sidebar.js';
import {popup} from '/ui/popup.js';
import {mount as mountModel} from '/ui/modules/model.js';
import {theme} from '/extensions/theme.js';
const controller=new AbortController();
const state={model:'test',models:[{name:'test',label:'Очень длинное название модели с подробным уточнением варианта и режима работы'}],reasoning:false,efforts:[]};
mountModel({root:document.querySelector('#model-menu'),signal:controller.signal,services:{'client.composer':{read:()=>state,subscribe:()=>()=>{},set:()=>{}}}});
const shadow=document.querySelector('#shadow-choice').attachShadow({mode:'open'}),style=document.createElement('style');style.textContent=theme;shadow.append(style);
const control=document.createElement('select');control.innerHTML='<option>A</option><option data-description="Пояснение внутри расширения">B</option>';shadow.append(control);
mountSidebar(document.querySelector('#sidebar'),()=>{});
const modelDetails=document.querySelector('.composer-model-menu');modelDetails.open=true;
await new Promise(resolve=>setTimeout(resolve,30));
window.probe=()=>{
    const result=[];
    const measure=(name,element,maxHeight)=>{
        const r=element.getBoundingClientRect();
        result.push({name,height:r.height,width:r.width});
        if(r.height<=0||r.height>maxHeight||r.width>300||r.top<0||r.bottom>innerHeight)
            throw Error(name+': '+JSON.stringify(r));
        for(const button of element.querySelectorAll('button'))
            if(button.getBoundingClientRect().height>64)throw Error(name+': stretched row');
    };
    const choiceBounds=element=>{
        const r=element.getBoundingClientRect();
        if(r.height<40 || r.height>400 || r.left<8 || r.right>innerWidth-8 || r.top<8 || r.bottom>innerHeight-8 || element.scrollWidth>element.clientWidth)throw Error('Choice bounds: '+JSON.stringify(r));
    };
    const modelPanel=document.querySelector('.composer-menu-panel');choiceBounds(modelPanel);
    if(modelPanel.querySelector('.choice-title').getBoundingClientRect().height<35)throw Error('Composer title clipped');
    modelPanel.hidePopover();modelDetails.open=false;
    const select=document.querySelector('#choices select');select.click();
    const picker=document.querySelector('.select-picker');choiceBounds(picker);
    if(picker.querySelector('.choice-title').getBoundingClientRect().height<35 || !picker.querySelector('.choice-description'))throw Error('Long choice lost text/hint');
    picker.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
    control.click();const shadowPicker=shadow.querySelector('.select-picker');choiceBounds(shadowPicker);
    if(getComputedStyle(shadowPicker).backgroundColor!==getComputedStyle(picker).backgroundColor || !shadowPicker.querySelector('.choice-description'))throw Error('Shadow picker lost shared theme');
    shadowPicker.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
    const project=document.querySelector('[data-workspace]');
    for(const top of [220,innerHeight-40]){
        project.style.top=top+'px';
        project.querySelector('[data-sidebar-menu]').click();
        const menu=document.querySelector('.sidebar-menu');
        measure('project@'+top,menu,360);menu.hidePopover();
    }
    document.querySelector('[data-session-dir] [data-sidebar-menu]').click();
    measure('session',document.querySelector('.sidebar-menu'),360);
    document.querySelector('.sidebar-menu').hidePopover();
    const tooltip=popup('sidebar-hover','Сведения');
    const title=document.createElement('strong');title.textContent='Чат';
    const detail=document.createElement('p');detail.textContent='4 сообщения · сейчас';
    tooltip.show([title,detail],project,undefined,false);
    measure('hover',tooltip.element,110);tooltip.dispose();
    return result;
};
</script></body></html>'''


class Assets(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path != '/probe.html':
            return super().do_GET()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.end_headers()
        self.wfile.write(PAGE.encode())


def main(label='WebKitGTK project/session menus; long select descriptions, bottom-edge geometry and Shadow DOM theme'):
    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Assets, directory=str(ROOT)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryFile(mode='w+') as log:
        display = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '1440x1000x24', '-nolisten', 'tcp'], stdout=subprocess.PIPE, stderr=log, text=True)
        try:
            number = display.stdout.readline().strip()
            assert number.isdecimal(), 'Xvfb did not start'
            os.environ.update(DISPLAY=':'+number, GDK_BACKEND='x11', WEBKIT_DISABLE_COMPOSITING_MODE='1')
            os.environ.pop('WAYLAND_DISPLAY', None)
            import gi
            gi.require_version('Gtk', '3.0')
            gi.require_version('WebKit2', '4.1')
            from gi.repository import Gtk, WebKit2, GLib
            window = Gtk.Window()
            window.set_default_size(1440, 1000)
            view = WebKit2.WebView()
            window.add(view)
            window.show_all()
            results, errors = [], []

            def done(view, task, data):
                try:
                    value = view.evaluate_javascript_finish(task).to_json(0)
                    if value and value != 'null':
                        results.extend(json.loads(value))
                        Gtk.main_quit()
                except Exception as error:
                    errors.append(str(error))
                    Gtk.main_quit()

            def probe():
                view.evaluate_javascript('window.probe ? window.probe() : null', -1, None, None, None, done, None)
                return not (results or errors)

            def timeout():
                errors.append('WebKit popup probe timed out')
                Gtk.main_quit()
                return False

            GLib.timeout_add(250, probe)
            GLib.timeout_add_seconds(15, timeout)
            view.load_uri(f'http://127.0.0.1:{server.server_port}/probe.html')
            try:
                Gtk.main()
            finally:
                window.destroy()
            assert not errors, '\n'.join(errors)
            assert len(results) == 4, results
            print('PASS: '+label+':', json.dumps(results))
        finally:
            display.terminate()
            display.wait(timeout=5)
            server.shutdown()


if __name__ == '__main__':
    main()
