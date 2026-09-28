const key='proteus.animations';
export const motionAllowed=()=>document.documentElement.dataset.animations!=='off'&&!matchMedia('(prefers-reduced-motion: reduce)').matches;
export function applyMotion(){
  let enabled=true;try{enabled=localStorage.getItem(key)!=='false';}catch{}
  document.documentElement.dataset.animations=enabled?'on':'off';
  if(!enabled)for(const animation of document.getAnimations())animation.cancel();
  window.dispatchEvent(new Event('proteus-motion-change'));
}
applyMotion();window.addEventListener('storage',event=>{if(event.key===key)applyMotion();});
document.addEventListener('change',event=>{if(event.target.matches('[data-animation-toggle]'))queueMicrotask(applyMotion);});
