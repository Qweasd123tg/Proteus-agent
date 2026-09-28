"""Three disclosure levels preserve the actual tool card and its state."""
import base64
from pathlib import Path


def run(command, js, wait_for):
    wait_for(lambda: js("return !!document.querySelector('.tool-chain .tool-card')"), 'Tool chain missing')
    assert js("const c=document.querySelector('.tool-chain');return !c.classList.contains('expanded') && c.getBoundingClientRect().height<=40 && c.querySelector('.tool-chain-items').hidden"), 'Compact chain is not one row'
    js("window.chain=document.querySelector('.tool-chain');window.card=chain.querySelector('.tool-card');chain.querySelector('.tool-chain-toggle').click()")
    assert js("return !chain.querySelector('.tool-chain-items').hidden && !card.classList.contains('expanded')"), 'Chain did not reveal brief calls'
    # Exercise adjacent nodes with the real rendered card markup. The provider
    # fixture emits one call, so add two brief neighbours for rail geometry.
    js("window.railRows=[card.parentElement];for(let i=0;i<2;i++){const row=railRows[0].cloneNode(true);railRows[0].parentElement.append(row);railRows.push(row)}")
    def connected():
        return js("return railRows.slice(0,-1).every((row,i)=>{const line=getComputedStyle(row,'::after'),dot=getComputedStyle(railRows[i+1],'::before');const end=row.getBoundingClientRect().bottom-parseFloat(line.bottom);const next=railRows[i+1].getBoundingClientRect().top+parseFloat(dot.top)+parseFloat(dot.height)/2;return Math.abs(end-next)<1}) && getComputedStyle(railRows.at(-1),'::after').content==='none'")
    assert connected(), 'Tool connectors overshoot the next node or leave a tail'
    wait_for(lambda: js("return !chain.querySelector('.tool-chain-items').getAnimations().length"), 'Chain reveal did not settle')
    Path('/tmp/proteus-tool-chain-brief.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("card.querySelector('.tool-card-summary').click()")
    wait_for(lambda: js("return !!card.querySelector('.tool-card-details') && card.querySelector('.tool-card-summary').getAttribute('aria-expanded')==='true'"), 'Call details missing')
    assert connected(), 'Opening call details broke the connection to the next call'
    js("chain.querySelector('.tool-chain-toggle').click();chain.querySelector('.tool-chain-toggle').click()")
    assert js("return chain.querySelector('.tool-card')===card && card.classList.contains('expanded')"), 'Collapse remounted or reset call'
    Path('/tmp/proteus-tool-chain.png').write_bytes(base64.b64decode(command('/screenshot', None)))
    js("railRows.slice(1).forEach(row=>row.remove());chain.querySelector('.tool-chain-toggle').click()")
    assert js("return !!document.querySelector('.role-assistant .message') && chain.getBoundingClientRect().height<=40"), 'Chain swallowed the reply'
    print('PASS: tool chain one row -> brief calls -> details; DOM/disclosure retained; reply outside chain', flush=True)
