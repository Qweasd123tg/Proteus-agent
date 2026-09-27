"""Inspector tools picker against a real config-builder snapshot and save/reload."""

import tomllib
import json
from urllib.parse import urlencode


def run(command, js, wait_for, web, origin, config, request):
    url = web + '/configs?' + urlencode({'server': origin, 'token': 'extension-smoke'})

    def enabled_names():
        return set(tomllib.loads(config.read_text())['tools']['enabled'])

    def row_state(name):
        return js('''
            const row = [...document.querySelectorAll('.tools-picker-row')]
                .find(row => row.querySelector('strong')?.textContent === arguments[0]);
            if (!row) return null;
            const input = row.querySelector('input[type=checkbox]');
            return {checked: input.checked, disabled: input.disabled,
                managed: row.textContent.includes('Управляется runtime')};
        '''.replace('arguments[0]', repr(name)))

    def open_picker():
        command('/url', {'url': url})
        wait_for(lambda: js("return !!document.querySelector('.cfg-tabs button') && !!document.querySelector('.tools-picker-row')"),
                 'Inspector config builder did not mount')
        js("[...document.querySelectorAll('.cfg-tabs button')].find(button => button.textContent === 'Инструменты').click()")

    def save(expected_selected):
        js("document.querySelector('.cfg-save-actions .btn-primary').click()")
        wait_for(lambda: ('update_plan' in enabled_names()) == expected_selected,
                 'Config builder did not persist the ordinary tool toggle')
        wait_for(lambda: js("return document.querySelector('.cfg-save-actions .btn-primary')?.disabled === true"),
                 'Config builder did not finish saving')
        names = enabled_names()
        assert 'fixture_managed' not in names, 'Picker added a managed tool to tools.enabled'

    open_picker()
    assert row_state('fixture_managed') == {'checked': True, 'disabled': True, 'managed': True}
    assert row_state('update_plan') == {'checked': True, 'disabled': False, 'managed': False}

    js("[...document.querySelectorAll('.cfg-filter-button')].find(button => button.textContent.includes('Включённые')).click()")
    assert row_state('fixture_managed')['checked'], 'Enabled filter hid runtime-managed tool'
    js("const input=document.querySelector('.tools-picker-head input[type=search]'); input.value='fixture_managed'; input.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: js("return document.querySelectorAll('.tools-picker-row').length === 1"),
             'Search did not narrow enabled tools')
    assert row_state('fixture_managed')['disabled']
    js("const input=document.querySelector('.tools-picker-head input[type=search]'); input.value=''; input.dispatchEvent(new Event('input',{bubbles:true}))")
    js("[...document.querySelectorAll('.cfg-filter-button')].find(button => button.textContent.includes('Все')).click()")

    js("[...document.querySelectorAll('.tools-picker-row')].find(row => row.querySelector('strong')?.textContent === 'update_plan').querySelector('input').click()")
    assert row_state('update_plan') == {'checked': False, 'disabled': False, 'managed': False}, 'Ordinary tool became managed after toggling'
    js("[...document.querySelectorAll('.tools-picker-row')].find(row => row.querySelector('strong')?.textContent === 'update_plan').querySelector('input').click()")
    assert row_state('update_plan') == {'checked': True, 'disabled': False, 'managed': False}
    js("[...document.querySelectorAll('.tools-picker-row')].find(row => row.querySelector('strong')?.textContent === 'update_plan').querySelector('input').click()")
    js("[...document.querySelectorAll('.cfg-filter-button')].find(button => button.textContent.includes('Включённые')).click()")
    assert row_state('update_plan') is None, 'Enabled filter retained toggled-off tool'
    assert row_state('fixture_managed')
    save(False)

    open_picker()
    assert row_state('update_plan') is None, 'Disabled builtin tool unexpectedly remained registered'
    assert row_state('fixture_managed') == {'checked': True, 'disabled': True, 'managed': True}
    assert 'fixture_managed' not in enabled_names()

    # Disabled builtin tools leave the runtime catalog after reload. Restore the
    # fixture through the same builder API so later browser checks keep their tool.
    session_dir = js('return sessionStorage.getItem(' + json.dumps('proteus.selectedSessionDir:' + origin) + ')')
    assert session_dir, 'Inspector did not keep the selected session'
    endpoint = origin + '/config/builder?' + urlencode({'token': 'extension-smoke', 'session_dir': session_dir})
    snapshot = request(endpoint)
    request(endpoint, 'POST', {
        'modules': {module['slot']: module['id'] for module in snapshot['active_modules']},
        'module_config': snapshot['module_config'],
        'tools_enabled': ['update_plan'],
        'active_provider': snapshot['active_provider'],
        'permission_mode': snapshot['permission_mode'],
    })
    wait_for(lambda: 'update_plan' in enabled_names(), 'Fixture tool was not restored')
    print('PASS: Inspector managed/configured tools, filter, search and save/reload', flush=True)
