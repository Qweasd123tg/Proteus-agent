import { motionAllowed } from '../ui/motion.js';
// Pointer drag previews the insertion position; only dropping writes the order.
export function enableReorder(list, registry, signal, status) {
  let drag;
  const rows=()=>[...list.children];
  function finish(commit){
    if(!drag)return;
    const current=drag;drag=null;current.ghost?.remove();current.row.classList.remove('drag-placeholder');
    if(current.started&&commit){
      const index=rows().indexOf(current.row);registry.move(current.id,index-current.index);
      status.textContent=`Позиция ${index+1}`;
      list.querySelector(`[data-reorder="${CSS.escape(current.id)}"]`)?.focus();
    }else if(current.started&&current.row.isConnected){for(const row of current.order)list.append(row);}
  }
  signal.addEventListener('abort',()=>finish(false),{once:true});
  list.addEventListener('pointerdown',event=>{
    const handle=event.target.closest('[data-reorder]');if(!handle||handle.disabled||event.button!==0)return;
    event.preventDefault();finish(false);
    const row=handle.closest('.extension-choice'),rect=row.getBoundingClientRect();
    drag={id:handle.dataset.reorder,row,index:rows().indexOf(row),order:rows(),pointer:event.pointerId,x:event.clientX,y:event.clientY,offset:event.clientY-rect.top,rect,started:false};
    list.setPointerCapture(event.pointerId);
  },{signal});
  list.addEventListener('pointermove',event=>{
    if(!drag||event.pointerId!==drag.pointer)return;
    if(!drag.started&&Math.hypot(event.clientX-drag.x,event.clientY-drag.y)<5)return;
    if(!drag.row.isConnected){finish(false);return;}
    if(!drag.started){
      drag.started=true;drag.ghost=drag.row.cloneNode(true);drag.ghost.classList.add('extension-drag-ghost');drag.ghost.setAttribute('aria-hidden','true');drag.ghost.inert=true;
      Object.assign(drag.ghost.style,{width:`${drag.rect.width}px`,left:`${drag.rect.left}px`});document.body.append(drag.ghost);drag.row.classList.add('drag-placeholder');
    }
    drag.ghost.style.top=`${event.clientY-drag.offset}px`;
    const before=rows().filter(row=>row!==drag.row).find(row=>{const top=list.getBoundingClientRect().top+row.offsetTop;return event.clientY<top+row.offsetHeight/2;});
    if((before??null)!==drag.row.nextElementSibling){
      const positions=new Map(rows().map(row=>[row,row.getBoundingClientRect().top]));
      list.insertBefore(drag.row,before??null);
      if(motionAllowed())for(const row of rows()){
        if(row===drag.row)continue;const delta=positions.get(row)-row.getBoundingClientRect().top;
        if(delta){for(const animation of row.getAnimations())animation.cancel();row.animate([{transform:`translateY(${delta}px)`},{transform:'translateY(0)'}],{duration:160,easing:'ease-out'});}
      }
    }
    const scroll=list.closest('.extension-settings-sidebar, .settings-content');if(scroll){const r=scroll.getBoundingClientRect();if(event.clientY>r.bottom-48)scroll.scrollTop+=18;else if(event.clientY<r.top+48)scroll.scrollTop-=18;}
  },{signal});
  list.addEventListener('pointerup',()=>finish(true),{signal});
  list.addEventListener('pointercancel',()=>finish(false),{signal});
  list.addEventListener('lostpointercapture',()=>finish(false),{signal});
  window.addEventListener('blur',()=>finish(false),{signal});
  document.addEventListener('keydown',event=>{if(event.key==='Escape'&&drag){event.preventDefault();event.stopImmediatePropagation();finish(false);}},{signal,capture:true});
  list.addEventListener('keydown',event=>{
    const handle=event.target.closest('[data-reorder]');if(!handle||!['ArrowUp','ArrowDown'].includes(event.key))return;
    event.preventDefault();const id=handle.dataset.reorder;registry.move(id,event.key==='ArrowUp'?-1:1);list.querySelector(`[data-reorder="${CSS.escape(id)}"]`)?.focus();status.textContent='Порядок изменён';
  },{signal});
}
