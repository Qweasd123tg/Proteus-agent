"""Three disclosure levels preserve the actual tool card and its state."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    wait_for(lambda: js("return !!document.querySelector('.tool-chain .tool-card')"), 'Tool chain missing')
    assert js("const c=document.querySelector('.tool-chain');return !c.classList.contains('expanded') && c.getBoundingClientRect().height<=40 && c.querySelector('.tool-chain-items').hidden"), 'Compact chain is not one row'
    js("window.chain=document.querySelector('.tool-chain');window.card=chain.querySelector('.tool-card');chain.querySelector('.tool-chain-toggle').click()")
    assert js("return !chain.querySelector('.tool-chain-items').hidden && !card.classList.contains('expanded')"), 'Chain did not reveal brief calls'
    js("card.querySelector('.tool-card-summary').click()")
    wait_for(lambda: js("return !!card.querySelector('.tool-card-details') && card.querySelector('.tool-card-summary').getAttribute('aria-expanded')==='true'"), 'Call details missing')
    js("chain.querySelector('.tool-chain-toggle').click();chain.querySelector('.tool-chain-toggle').click()")
    assert js("return chain.querySelector('.tool-card')===card && card.classList.contains('expanded')"), 'Collapse remounted or reset call'
    Path('/tmp/proteus-tool-chain.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("chain.querySelector('.tool-chain-toggle').click()")
    assert js("return !!document.querySelector('.role-assistant .message') && chain.getBoundingClientRect().height<=40"), 'Chain swallowed the reply'
    print('PASS: tool chain one row -> brief calls -> details; DOM/disclosure retained; reply outside chain', flush=True)
