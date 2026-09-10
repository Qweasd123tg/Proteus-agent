"""Real journal → authenticated usage API → sidebar, settings and context report."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    def shadow():
        return "document.querySelector('[data-extension-id=usage] .extension-panel-content')?.shadowRoot"

    def chat_loaded():
        return js("return !!document.querySelector('.composer textarea')")

    js("document.querySelector('.settings-link').click()")
    wait_for(lambda: js("return !!document.querySelector('[data-extension-available=usage]')"), 'Usage package unavailable in saved settings')
    js("document.querySelector('[data-extension-available=usage]').click()")
    wait_for(lambda: js("return !!document.querySelector('[aria-label=\"Настроить: Расход\"]')"), 'Usage settings action missing')
    assert js("return !document.querySelector('.extension-options-content')"), 'Settings entry executed before opening'
    js("document.querySelector('[aria-label=\"Настроить: Расход\"]').click()")
    wait_for(lambda: js("return !!document.querySelector('.extension-options-content')?.shadowRoot?.querySelector('form')"), 'Separate settings entry did not mount')
    js("const root=document.querySelector('.extension-options-content').shadowRoot;for(const [name,value] of Object.entries({provider:'',model:'fixture-model',input:1,cached:.1,write:1,output:2,threshold:0,input_multiplier:1,output_multiplier:1}))root.querySelector(`[name=${name}]`).value=value;root.querySelector('form').requestSubmit()")
    wait_for(lambda: js("return document.querySelector('.extension-options-content').shadowRoot.textContent.includes('Тариф сохранён')"), 'Custom rate failed to persist')
    js("document.querySelector('.topnav a[href=\"/\"]').click()")
    wait_for(chat_loaded, 'Chat did not return')
    wait_for(lambda: js("const root=" + shadow() + ";return root?.querySelector('.usage-headline > strong')?.textContent==='280'"), 'Usage totals do not match two real provider requests')
    assert js("const root=" + shadow() + ";return root.querySelector('.cost-total').textContent.includes('$0.000252')"), 'Cost double-counted cache or reasoning'
    js("const root=" + shadow() + ";root.querySelector('.usage-recent').open=true;root.querySelector('.usage-request').open=true")
    assert js("const root=" + shadow() + ";return root.querySelectorAll('.usage-request').length===2 && root.textContent.includes('Рассуждения') && root.textContent.includes('Пользовательский')"), 'Per-request details missing'
    js("document.querySelector('[data-extension-id=usage]').scrollIntoView({block:'start'})")
    Path('/tmp/proteus-usage-panel.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("document.querySelector('.topnav a[href=\"/context\"]').click()")
    wait_for(lambda: js("return document.querySelector('.usage-details-host > div')?.shadowRoot?.querySelectorAll('.usage-request').length===2"), 'Context report did not load real journal')
    assert js("const root=document.querySelector('.usage-details-host > div').shadowRoot;return root.querySelector('.cost-total').textContent.includes('$0.000252') && root.querySelectorAll('.request-cell').length===10"), 'Context and sidebar reports disagree'
    js("const root=document.querySelector('.usage-details-host > div').shadowRoot;root.querySelector('.usage-request').open=true;document.querySelector('.context-usage-report').scrollIntoView({block:'start'})")
    Path('/tmp/proteus-usage-context.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    command('/refresh', {})
    wait_for(lambda: js("return document.querySelector('.usage-details-host > div')?.shadowRoot?.querySelector('.cost-total')?.textContent.includes('$0.000252')"), 'Reload lost usage or custom pricing')
    # Real HTTP failure must clear old totals, then recover from the journal.
    js("window.usageFetch=window.fetch;window.fetch=(input,init)=>String(input.url||input).includes('/usage')?Promise.resolve(new Response('offline',{status:503})):window.usageFetch(input,init);document.querySelector('.usage-details-host > div').shadowRoot.querySelector('.usage-footer button').click()")
    wait_for(lambda: js("const root=document.querySelector('.usage-details-host > div').shadowRoot;return root.textContent.includes('Не удалось получить расход') && !root.querySelector('.cost-total')"), 'Usage failure retained a stale total')
    js("window.fetch=window.usageFetch;document.querySelector('.usage-details-host > div').shadowRoot.querySelector('.usage-footer button').click()")
    wait_for(lambda: js("return document.querySelector('.usage-details-host > div').shadowRoot.querySelectorAll('.usage-request').length===2"), 'Usage recovery failed')
    js("document.querySelector('.topnav a[href=\"/\"]').click()")
    wait_for(chat_loaded, 'Chat did not restore after context report')
    # Keep the layout regression focused on the original visible panels.
    wait_for(lambda: js("return !!document.querySelector('[data-extension-id=usage] .extension-panel-title')"), 'Usage panel not restored')
    js("document.querySelector('[data-extension-id=usage] .extension-panel-title').click()")
    print('PASS: durable per-request usage; cache/reasoning pricing; independent rate settings; sidebar/context agreement; reload and API error recovery', flush=True)
