"""Themed select pickers preserve real pointer, keyboard and Shadow DOM behavior."""


def run(command, js, wait_for):
    def pointer(expression):
        x, y = js(f"const r=({expression}).getBoundingClientRect();return [Math.round(r.x+r.width/2),Math.round(r.y+r.height/2)]")
        command('/actions', {'actions': [{'type': 'pointer', 'id': 'select-pointer', 'parameters': {'pointerType': 'mouse'}, 'actions': [
            {'type': 'pointerMove', 'duration': 0, 'origin': 'viewport', 'x': x, 'y': y},
            {'type': 'pointerDown', 'button': 0}, {'type': 'pointerUp', 'button': 0}]}]})

    def keys(*values):
        command('/actions', {'actions': [{'type': 'key', 'id': 'select-keyboard', 'actions': [
            action for value in values for action in ({'type': 'keyDown', 'value': value}, {'type': 'keyUp', 'value': value})]}]})

    select = "document.querySelector('#picker-fixture select')"
    menu = "document.querySelector('.select-picker')"
    try:
        js("""
          const fixture=document.createElement('div');fixture.id='picker-fixture';
          fixture.style.cssText='position:fixed;top:80px;left:100px;z-index:20000;background:#222;padding:10px;display:flex;gap:10px';
          fixture.innerHTML='<button id="picker-before">Before</button><select aria-label="Fixture choice"><option value="alpha">Alpha</option><option disabled>Disabled</option><optgroup label="Unavailable" disabled><option>Grouped disabled</option></optgroup><optgroup label="Hidden group" hidden><option>Hidden</option></optgroup><optgroup label="Available"><option value="beta">Beta</option><option value="gamma">Gamma</option></optgroup></select><button id="picker-after">After</button><div id="picker-shadow"></div>';
          fixture.querySelector('select').addEventListener('input',()=>fixture.dataset.inputs=String(+(fixture.dataset.inputs||0)+1));
          fixture.querySelector('select').addEventListener('change',()=>fixture.dataset.changes=String(+(fixture.dataset.changes||0)+1));
          document.body.append(fixture);
        """)
        pointer(select)
        wait_for(lambda: js(f"return !!{menu} && document.activeElement==={menu}"), 'Trusted pointer did not open and focus themed picker')
        assert js(f"return {menu}.matches(':popover-open') && {select}.getAttribute('aria-expanded')==='true'"), 'Picker is not above the page in the popover layer'
        assert js(f"return [...{menu}.querySelectorAll('.select-picker-group')].map(n=>n.textContent).join(',')==='Unavailable,Available'"), 'Optgroup headings are missing or hidden group is exposed'
        pointer(f"{menu}.querySelectorAll('[role=option]')[1]")
        assert js(f"return !!{menu} && {select}.value==='alpha'"), 'Disabled pointer option was selected'
        keys('\ue015', '\ue007')  # ArrowDown, Enter skips disabled and hidden groups.
        assert js(f"return !{menu} && {select}.value==='beta' && document.activeElement==={select} && document.querySelector('#picker-fixture').dataset.inputs==='1' && document.querySelector('#picker-fixture').dataset.changes==='1'"), 'Keyboard selection lost value, bindings or trigger focus'
        keys(' ')
        wait_for(lambda: js(f"return !!{menu}"), 'Space did not open themed picker')
        keys('\ue010', '\ue00c')  # End, Escape must not commit.
        assert js(f"return !{menu} && {select}.value==='beta' && document.activeElement==={select}"), 'Escape committed or lost focus'
        keys('g', '\ue007')
        assert js(f"return {select}.value==='gamma'"), 'Typing from the trigger did not select through the themed menu'
        pointer(select)
        keys('\ue004')  # Tab continues from the trigger in DOM tab order.
        assert js(f"return !{menu} && document.activeElement.id==='picker-after'"), 'Tab trapped focus or skipped the next field'
        pointer(select)
        command('/actions', {'actions': [{'type': 'key', 'id': 'select-keyboard', 'actions': [
            {'type': 'keyDown', 'value': '\ue008'}, {'type': 'keyDown', 'value': '\ue004'},
            {'type': 'keyUp', 'value': '\ue004'}, {'type': 'keyUp', 'value': '\ue008'}]}]})
        assert js(f"return !{menu} && document.activeElement.id==='picker-before'"), 'Shift+Tab did not return to the previous field'
        pointer(select)
        pointer("document.querySelector('#picker-after')")
        assert js(f"return !{menu} && document.activeElement.id==='picker-after'"), 'Outside click did not dismiss and transfer focus'
        pointer(select)
        pointer(f"{menu}.querySelectorAll('[role=option]')[0]")
        assert js(f"return !{menu} && {select}.value==='alpha'"), 'Trusted pointer did not commit a selectable option'
        pointer(select)
        js(f"{select}.remove()")
        wait_for(lambda: js(f"return !{menu}"), 'Detached trigger left an orphan menu')
        js("""
          const root=document.querySelector('#picker-shadow').attachShadow({mode:'open'});
          const style=document.createElement('style');style.textContent='.select-picker{position:fixed;inset:auto;margin:0;padding:5px;background:#222;color:white;max-height:320px;overflow:auto}.select-picker-option{padding:8px}';root.append(style);
          const select=document.createElement('select');select.innerHTML='<option>A</option><option>B</option>';root.append(select);
        """)
        shadow = "document.querySelector('#picker-shadow').shadowRoot"
        pointer(f"{shadow}.querySelector('select')")
        wait_for(lambda: js(f"return !!{shadow}.querySelector('.select-picker') && {shadow}.activeElement?.classList.contains('select-picker')"), 'Shadow DOM select did not open its themed picker')
        keys('\ue015', '\ue007')
        assert js(f"return {shadow}.querySelector('select').value==='B' && !{shadow}.querySelector('.select-picker')"), 'Shadow DOM keyboard binding failed'
        pointer(f"{shadow}.querySelector('select')")
        js("window.detachedPickerRoot=document.querySelector('#picker-shadow').shadowRoot;document.querySelector('#picker-shadow').remove()")
        wait_for(lambda: js("return !window.detachedPickerRoot.querySelector('.select-picker')"), 'Detached Shadow DOM host retained menu and listeners')
    finally:
        js("document.querySelector('#picker-fixture')?.remove();delete window.detachedPickerRoot")
    print('PASS: themed select trusted pointer; groups; value events; keyboard; Escape; Tab; outside click; Shadow DOM cleanup', flush=True)
