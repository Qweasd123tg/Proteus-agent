"""Agent settings: per-slot pages share one draft and save the real profile."""
import json


def run(command, js, wait_for, config, capture):
    def click(selector):
        element = command('/element', {'using': 'css selector', 'value': selector})
        command('/element/' + element['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    def page(id):
        click(f'[data-settings-section={id}]')
        wait_for(lambda: js(f"const s=document.querySelector('[data-module-page={id}]');return s && !s.hidden && !!s.querySelector('.agent-save-bar')"), 'Agent page missing: ' + id)

    def status():
        return js("return [...document.querySelectorAll('.settings-section:not([hidden]) .agent-save-status')].map(x=>x.textContent).join('')")

    click('.settings-link')
    wait_for(lambda: js("return !!document.querySelector('[data-settings-section=agent-workflow]')"), 'Agent settings missing')
    assert js("return [...document.querySelectorAll('.settings-nav-label')].map(x=>x.textContent).join('|')") == 'Агент|Интерфейс|Диагностика', 'Settings blocks are not Agent / Interface / Diagnostics'
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
    wait_for(lambda: 'Инструменты' in status(), status())
    capture('tools-dirty')
    before = config.read_text()
    click('[data-module-page=agent-tools] [data-agent-save]')
    wait_for(lambda: status().startswith('Сохранено'), 'Profile was not saved: ' + status())
    saved = config.read_text()
    assert saved != before and 'mode = "plan"' in saved and 'enabled = []' in saved, saved
    page('agent-access')
    assert js("return document.querySelector('[data-agent-mode=plan] input').checked"), 'Other pages missed the saved profile'
    capture('access')
    page('agent-model')
    wait_for(lambda: js("return !!document.querySelector('[data-agent-provider] input:checked') && !!document.querySelector('[data-agent-parameters=\"model/custom-model\"] [data-parameter=implementation]')"), 'Model parameters missing')
    capture('model')
    print('PASS: agent pages share one draft, reject invalid parameters and save the profile: '
          + json.dumps({'saved_bytes': len(saved)}), flush=True)
