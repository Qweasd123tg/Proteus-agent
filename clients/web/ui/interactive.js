import {interactive} from './markdown-loaders.js';

export async function renderInteractive(code) {
  const {render}=await interactive();
  if(!code.isConnected)return;
  const view=render(code.textContent),block=code.closest('.code-block'),pre=code.parentElement;
  block.insertBefore(view,pre);pre.hidden=true;
  const toggle=document.createElement('button');toggle.type='button';toggle.className='code-source';toggle.textContent='Код';toggle.setAttribute('aria-expanded','false');
  toggle.addEventListener('click',()=>{pre.hidden=!pre.hidden;toggle.setAttribute('aria-expanded',String(!pre.hidden));});
  block.querySelector('.code-actions').prepend(toggle);
}
