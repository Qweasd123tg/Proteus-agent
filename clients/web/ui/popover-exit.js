// A closed popover leaves the top layer in current Firefox/WebKit. Containment
// would change its fixed-position origin, so only its inert pixels fade on body.
export function needsExitSnapshot(element) {
  if (CSS.supports('overlay', 'auto')) return false;
  for (let parent = element.parentElement || element.getRootNode().host; parent;
    parent = parent.parentElement || parent.getRootNode().host) {
    const style = getComputedStyle(parent);
    if (/(layout|paint|strict|content)/.test(style.contain) || style.containerType !== 'normal' ||
        ['transform', 'translate', 'rotate', 'scale', 'filter', 'perspective'].some(key => style[key] && style[key] !== 'none')) return true;
  }
  return false;
}

export function exitSnapshot(source) {
  // This visual path is for menus, never live embedded views or media.
  if (source.querySelector('iframe,object,embed,video,audio,canvas,[role="tabpanel"]')) return null;
  const rect = source.getBoundingClientRect(), style = getComputedStyle(source);
  const opacity = style.opacity, sourceY = parseFloat(style.translate.split(' ')[1]) || 0;
  const rawDuration = style.getPropertyValue('--motion-exit').trim() || '140ms';
  const duration = parseFloat(rawDuration) * (rawDuration.endsWith('ms') ? 1 : 1000);
  const easing = style.getPropertyValue('--motion-ease').trim() || 'ease';
  const copy = source.cloneNode(true), originals = [source, ...source.querySelectorAll('*')];
  const copies = [copy, ...copy.querySelectorAll('*')];
  copies.forEach((node, index) => {
    const computed = getComputedStyle(originals[index]);
    node.style.cssText = [...computed].map(key => `${key}:${computed.getPropertyValue(key)};`).join('');
    for (const attribute of [...node.attributes]) {
      const visualState = /^aria-(selected|checked|disabled|pressed|expanded)$/.test(attribute.name);
      if (['id', 'role', 'name', 'for', 'tabindex', 'popover', 'autofocus'].includes(attribute.name) || /^(data-|on)/.test(attribute.name) || attribute.name.startsWith('aria-') && !visualState) node.removeAttribute(attribute.name);
    }
    node.style.setProperty('transition', 'none', 'important');
    node.style.setProperty('animation', 'none', 'important');
  });
  copy.className = 'ui-popover-exit';
  copy.inert = true;
  copy.setAttribute('aria-hidden', 'true');
  Object.assign(copy.style, {
    position:'fixed', inset:'auto', left:`${rect.left}px`, top:`${rect.top}px`,
    width:`${rect.width}px`, height:`${rect.height}px`, minWidth:'0', minHeight:'0',
    maxWidth:'none', maxHeight:'none', margin:'0', boxSizing:'border-box',
    transform:'none', translate:'none', rotate:'none', scale:'none', opacity,
    pointerEvents:'none', zIndex:'2147483000',
  });
  document.body.append(copy);
  copies.forEach((node, index) => {
    node.scrollTop = originals[index].scrollTop;
    node.scrollLeft = originals[index].scrollLeft;
  });
  const animation = copy.animate([{opacity, translate:'0 0'}, {opacity:0, translate:'0 -4px'}], {duration, easing, fill:'forwards'});
  return {
    finished: animation.finished.catch(() => {}),
    current() {
      const current = getComputedStyle(copy);
      return {opacity:current.opacity, translate:`0 ${sourceY + (parseFloat(current.translate.split(' ')[1]) || 0)}px`};
    },
    stop() { animation.cancel(); copy.remove(); },
  };
}
