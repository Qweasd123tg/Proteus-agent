"""Agent settings: per-slot pages share one draft and save the real profile."""
import json
import tomllib


def run(command, js, wait_for, config, capture):
    def click(selector):
        js(f"document.querySelector({json.dumps(selector)}).scrollIntoView({{block:'center',inline:'nearest',behavior:'instant'}})")
        element = command('/element', {'using': 'css selector', 'value': selector})
        command('/element/' + element['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    def page(id):
        click(f'[data-settings-section={id}]')
        wait_for(lambda: js(f"const s=document.querySelector('[data-module-page={id}]');return s && !s.hidden && !!s.querySelector('.agent-save-bar')"), 'Agent page missing: ' + id)

    def status():
        return js("return [...document.querySelectorAll('.settings-section:not([hidden]) .agent-save-status')].map(x=>x.textContent).join('')")

    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=agent-workflow]')"), 'Agent settings missing')
    assert js("return [...document.querySelectorAll('.settings-nav-label')].map(x=>x.textContent).join('|')") == 'Агент|Встроенные|Расширения', 'Settings blocks are not Agent / Builtins / Extensions'
    assert js("return !document.querySelector('[data-builtin-module^=agent-]')"), 'Agent pages became optional interface modules'

    page('agent-workflow')
    wait_for(lambda: js("return !!document.querySelector('[data-agent-module=\"coding.single_loop\"] input:checked')"), 'Active workflow is not selected')
    assert status() == 'Изменений нет', status()
    # Broken JSON stays in the draft, is reported and blocks the save.
    click('[data-module-page=agent-workflow] .agent-mode-toggle')
    js("const area=document.querySelector('[data-module-page=agent-workflow] .agent-raw');area.value='{ broken';area.dispatchEvent(new Event('input',{bubbles:true}))")
    wait_for(lambda: status() == 'Исправьте отмеченные значения', 'Invalid parameters were not reported: ' + status())
    assert js("return document.querySelector('[data-module-page=agent-workflow] [data-agent-save]').disabled"), 'Invalid parameters can be saved'
    capture('workflow-invalid')
    click('[data-module-page=agent-workflow] .agent-save-actions button:nth-child(2)')
    wait_for(lambda: status() == 'Изменений нет', 'Reset did not restore the profile: ' + status())

    page('agent-access')
    click('[data-agent-mode=plan] input')
    wait_for(lambda: status() == 'Не сохранено: Режим прав', status())
    page('agent-tools')
    wait_for(lambda: status() == 'Не сохранено: Режим прав', 'Pages do not share one draft: ' + status())
    click('[data-agent-tool=update_plan] input')
    wait_for(lambda: 'Tools' in status(), status())
    capture('tools-dirty')
    before = config.read_text()
    click('[data-module-page=agent-tools] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'Profile was not saved: ' + status())
    saved = config.read_text()
    assert saved != before and 'mode = "plan"' in saved and 'enabled = []' in saved, saved
    page('agent-access')
    assert js("return document.querySelector('[data-agent-mode=plan] input').checked"), 'Other pages missed the saved profile'
    capture('access')
    # History keeps the replaced state; rolling back is an ordinary save.
    page('agent-history')
    wait_for(lambda: js("return document.querySelectorAll('[data-agent-revision]').length===1"), 'Save did not record the replaced state')
    change = lambda key: js(f"return document.querySelector('[data-agent-revision] [data-change={key}]')?.textContent||''")
    assert change('mode') == 'Режим прав: По правилам → Только чтение', change('mode')
    assert change('tools') == 'Tools: − update_plan', change('tools')
    click('[data-agent-revision] button')
    wait_for(lambda: status() == 'Не сохранено: Tools, Режим прав', 'Rollback did not fill the draft: ' + status())
    assert js("return document.querySelector('[data-agent-revision] button').textContent") == 'В черновике'
    capture('history')
    click('[data-module-page=agent-history] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'Rollback was not saved: ' + status())
    restored = config.read_text()
    assert 'mode = "normal"' in restored and 'enabled = ["update_plan"]' in restored, restored
    wait_for(lambda: js("return [...document.querySelectorAll('[data-agent-revision] button')].map(x=>x.textContent).join('|')") == 'Вернуть как было|Текущее состояние', 'Rollback is not reversible in history')
    page('agent-model')
    wait_for(lambda: js("return !!document.querySelector('[data-agent-provider] input:checked') && !!document.querySelector('[data-agent-parameters=\"model/custom-model\"] [data-parameter=implementation]')"), 'Model parameters missing')
    capture('model')
    model = '[data-agent-parameters="model/custom-model"]'
    field = lambda key: model + ' [data-parameter="' + key + '"]'
    assert js(f"return document.querySelector('{field('prompt_cache')} input').checked"), 'Boolean default is not shown'
    assert js(f"return document.querySelector('{field('implementation')} select').selectedOptions[0].textContent") == 'Подписка ChatGPT'
    assert status() == 'Изменений нет' or status().startswith('Сохранено'), 'Displaying defaults edited the profile: ' + status()
    assert js("return document.querySelector('[data-module-page=agent-model] [data-agent-save]').disabled"), 'Displaying defaults enabled saving'
    click(field('prompt_cache') + ' input')
    click(model + ' > .agent-parameters-body > .agent-advanced > summary')
    def enter(selector, value):
        js(f"const input=document.querySelector('{selector}');input.value={json.dumps(value)};input.dispatchEvent(new Event('input',{{bubbles:true}}))")
    enter(field('request_max_retries') + ' input', '-1')
    wait_for(lambda: status() == 'Исправьте отмеченные значения', 'Invalid number was not reported')
    assert js(f"return document.querySelector('{field('request_max_retries')} .agent-error').textContent") == 'Минимум: 0'
    assert js("return document.querySelector('[data-module-page=agent-model] [data-agent-save]').disabled")
    enter(field('request_max_retries') + ' input', '9')
    click(field('capabilities') + ' > .agent-field-control .agent-object > summary')
    click(field('capabilities.supports_image_input') + ' input')
    capture('model-form')
    click('[data-module-page=agent-model] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'Typed form did not save: ' + status())
    values = tomllib.loads(config.read_text())['module_config']['model']['custom-model']
    assert values['prompt_cache'] is False and values['request_max_retries'] == 9
    assert values['capabilities'] == {'supports_image_input': True}, 'Nested defaults were materialized'
    assert 'max_input_tokens' not in values and 'http1_only' not in values, 'Visible defaults were materialized'
    # Each reset removes only its override, keeping unrelated values.
    click(field('prompt_cache') + ' > .agent-field-reset')
    click(model + ' > .agent-parameters-body > .agent-advanced > summary')
    click(field('request_max_retries') + ' > .agent-field-reset')
    click(field('capabilities') + ' > .agent-field-reset')
    assert js(f"return document.querySelector('{field('prompt_cache')} input').checked"), 'Reset did not restore the default'
    click('[data-module-page=agent-model] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'Field resets did not save')
    values = tomllib.loads(config.read_text())['module_config']['model']['custom-model']
    assert not any(key in values for key in ['prompt_cache', 'request_max_retries', 'capabilities'])
    assert 'base_url' in values and 'auth_file' in values
    page('agent-context')
    # Changing one inherited list item must retain all other default items.
    click('[data-agent-module=repo_aware] input')
    context = '[data-agent-parameters="context/repo_aware"]'
    wait_for(lambda: js(f"return !!document.querySelector('{context} [data-parameter=providers] .agent-array')"), 'Context list form missing')
    assert js(f"return document.querySelectorAll('{context} [data-parameter=providers] .agent-array-item').length") == 6
    click(context + ' [data-parameter=providers] .agent-array-item:last-child button')
    capture('context-form')
    click('[data-module-page=agent-context] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'List did not save: ' + status())
    values = tomllib.loads(config.read_text())['module_config']['context']['repo_aware']
    assert values == {'providers': ['project_instructions', 'manifest', 'git_status', 'repo_tree', 'memory']}, values
    # Tools name their host-reported plugin and pack; the link opens that pack.
    page('agent-tools')
    owner = js("const b=document.querySelector('[data-module-page=agent-tools] [data-agent-owner]');return b&&b.dataset.agentOwner")
    assert owner, 'Plugin tools show no owner link'
    plugin, pack = owner.split('/', 1)
    click(f'[data-module-page=agent-tools] [data-agent-owner="{owner}"]')
    scope = f'[data-module-page=agent-plugins] [data-agent-plugin="{plugin}"] [data-agent-pack="{pack}"]'
    wait_for(lambda: js(f"const s=document.querySelector('[data-module-page=agent-plugins]');const t=document.querySelector('{scope}');return s&&!s.hidden&&t&&t.contains(document.activeElement)"), 'Owner link did not show and focus its pack')
    capture('plugins-pack')
    tools = js(f"return [...document.querySelectorAll('{scope} [data-agent-pack-tool] input:not(:disabled)')].map(i=>i.closest('[data-agent-pack-tool]').dataset.agentPackTool)")
    assert len(tools) >= 2, 'Pack has too few switchable tools for a mixed state: ' + json.dumps(tools)
    group = scope + ' [data-agent-pack-toggle]'
    if js(f"return document.querySelector('{group}').checked"):
        click(group)
    click(group)
    wait_for(lambda: js(f"return document.querySelector('{scope}').dataset.state==='on'"), 'Pack group did not enable all tools')
    click(f'{scope} [data-agent-pack-tool="{tools[0]}"] input')
    wait_for(lambda: js(f"const g=document.querySelector('{group}');return document.querySelector('{scope}').dataset.state==='mixed'&&g.indeterminate&&!g.checked"), 'Partial pack is not mixed')
    assert 'Tools' in status(), status()
    page('agent-tools')
    assert 'Tools' in status(), 'Plugins page left the shared draft: ' + status()
    assert js(f"return !document.querySelector('[data-agent-tool=\"{tools[0]}\"] input').checked && document.querySelector('[data-agent-tool=\"{tools[1]}\"] input').checked"), 'Tools page does not follow pack edits'
    click('[data-module-page=agent-tools] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'Pack edits did not save: ' + status())
    enabled = tomllib.loads(config.read_text())['tools']['enabled']
    assert tools[0] not in enabled and all(name in enabled for name in tools[1:]), enabled
    # A narrow view retains controls inside the page rather than overflowing it.
    command('/window/rect', {'width': 900, 'height': 800})
    wait_for(lambda: js("const c=document.querySelector('.settings-content');return c.scrollWidth<=c.clientWidth+1 && c.getBoundingClientRect().right<=innerWidth+1"), 'Settings form overflows a narrow view')
    capture('context-narrow')
    command('/window/rect', {'width': 1440, 'height': 1000})
    print('PASS: shared draft and history; plugin pack provenance, group/mixed toggles; typed forms, validation, sparse nested/list persistence, field reset and narrow layout: '
          + json.dumps({'saved_bytes': len(saved)}), flush=True)
