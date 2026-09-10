// Visibility belongs to the viewport, including the assistant's reply to each prompt.
// IntersectionObserver tracks cards without reading the whole history on scroll.
export function mountMessageNav(root) {
  const workspace = root.closest('.session-workspace');
  const results = workspace.querySelector('.results-panel');
  const dock = workspace.querySelector('.composer');
  const track = root.querySelector('.msg-nav-track');
  const preview = root.querySelector('.msg-nav-preview');
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const targets = new Map(), counts = new Map();
  let ticks = [], observer, inset = -1, syncFrame = 0, hoverFrame = 0;
  let selected = null, pointerY = null, wave = new Set();

  function visible(tick, delta) {
    const count = Math.max(0, (counts.get(tick) || 0) + delta);
    counts.set(tick, count);
    tick.classList.toggle('is-visible', count > 0);
  }
  function resetObserver(nextInset) {
    observer?.disconnect();
    for (const [element, target] of targets) {
      if (target.visible) visible(target.tick, -1);
      targets.delete(element);
    }
    inset = nextInset;
    const nextObserver = new IntersectionObserver(entries => {
      if (observer !== nextObserver) return;
      for (const entry of entries) {
        const target = targets.get(entry.target);
        if (!target) continue;
        const next = entry.isIntersecting && entry.intersectionRect.height > 0;
        if (next === target.visible) continue;
        target.visible = next;
        visible(target.tick, next ? 1 : -1);
      }
    }, { root: results, rootMargin: `0px 0px -${inset}px 0px` });
    observer = nextObserver;
  }
  function sync() {
    syncFrame = 0;
    ticks = [...track.querySelectorAll('.msg-nav-tick')];
    if (ticks.length < 2) {
      observer?.disconnect();
      observer = null;
      inset = -1;
      for (const tick of counts.keys()) tick.classList.remove('is-visible');
      targets.clear();
      counts.clear();
      hide();
      return;
    }
    const current = new Set(ticks);
    if (selected && !current.has(selected)) hide();
    if (!ticks.some(tick => tick.tabIndex === 0) && ticks[0]) ticks[0].tabIndex = 0;
    const nextInset = Math.ceil(dock.getBoundingClientRect().height);
    if (inset !== nextInset) resetObserver(nextInset);
    const anchors = new Map(ticks.map(tick => [tick.dataset.messageId, tick]));
    const next = new Map();
    let owner;
    for (const card of results.children) {
      owner = anchors.get(card.id) || owner;
      if (owner) next.set(card, owner);
    }
    for (const [element, target] of targets) {
      if (next.get(element) === target.tick) continue;
      observer.unobserve(element);
      if (target.visible) visible(target.tick, -1);
      targets.delete(element);
    }
    for (const [element, tick] of next) {
      if (targets.has(element)) continue;
      targets.set(element, { tick, visible: false });
      observer.observe(element);
    }
    for (const tick of counts.keys()) if (!current.has(tick)) counts.delete(tick);
  }
  function scheduleSync() {
    if (!syncFrame) syncFrame = requestAnimationFrame(sync);
  }
  function clearWave() {
    for (const tick of wave) tick.style.removeProperty('--tick-wave');
    wave.clear();
  }
  function show(index, position = index) {
    const tick = ticks[index];
    if (!tick) return;
    const rect = track.getBoundingClientRect(), bounds = root.getBoundingClientRect();
    const y = rect.top - bounds.top + (index + .5) * rect.height / ticks.length;
    root.style.setProperty('--preview-y', `${Math.max(44, Math.min(bounds.height - 44, y))}px`);
    if (selected !== tick) {
      selected?.classList.remove('is-previewed');
      selected?.removeAttribute('aria-describedby');
      selected = tick;
      tick.classList.add('is-previewed');
      tick.setAttribute('aria-describedby', preview.id);
      preview.firstElementChild.textContent = tick.dataset.preview;
    }
    root.classList.add('preview-open');
    const nextWave = new Set();
    if (!motion.matches) {
      for (let i = Math.max(0, index - 4); i <= Math.min(ticks.length - 1, index + 4); i++) {
        const amount = 3 * Math.exp(-((i - position) ** 2) / 3);
        ticks[i].style.setProperty('--tick-wave', amount.toFixed(3));
        nextWave.add(ticks[i]);
      }
    }
    for (const old of wave) if (!nextWave.has(old)) old.style.removeProperty('--tick-wave');
    wave = nextWave;
  }
  function hide() {
    cancelAnimationFrame(hoverFrame);
    hoverFrame = 0;
    selected?.classList.remove('is-previewed');
    selected?.removeAttribute('aria-describedby');
    selected = null;
    root.classList.remove('preview-open');
    clearWave();
  }
  function pointerMove(event) {
    pointerY = event.clientY;
    if (hoverFrame) return;
    hoverFrame = requestAnimationFrame(() => {
      hoverFrame = 0;
      const rect = track.getBoundingClientRect();
      if (!rect.height || !ticks.length) return;
      const position = Math.max(0, Math.min(ticks.length - 1, (pointerY - rect.top) / rect.height * ticks.length - .5));
      show(Math.round(position), position);
    });
  }
  function pointerLeave() {
    const focused = ticks.indexOf(document.activeElement);
    if (focused >= 0) show(focused); else hide();
  }
  function focusIn(event) {
    const index = ticks.indexOf(event.target);
    if (index < 0) return;
    for (const tick of ticks) tick.tabIndex = tick === event.target ? 0 : -1;
    show(index);
  }
  function focusOut(event) {
    if (!root.contains(event.relatedTarget)) hide();
  }
  function keyDown(event) {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); hide(); return; }
    const index = ticks.indexOf(event.target);
    const next = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: ticks.length - 1 }[event.key];
    if (next === undefined || index < 0) return;
    event.preventDefault();
    ticks[Math.max(0, Math.min(ticks.length - 1, next))]?.focus({ preventScroll: true });
  }
  const mutations = new MutationObserver(scheduleSync);
  mutations.observe(track, { childList: true });
  mutations.observe(results, { childList: true });
  const resize = new ResizeObserver(scheduleSync);
  resize.observe(dock);
  resize.observe(results);
  track.addEventListener('pointermove', pointerMove);
  track.addEventListener('pointerleave', pointerLeave);
  track.addEventListener('focusin', focusIn);
  track.addEventListener('focusout', focusOut);
  track.addEventListener('keydown', keyDown);
  motion.addEventListener('change', hide);
  scheduleSync();
  return () => {
    cancelAnimationFrame(syncFrame);
    hide();
    mutations.disconnect();
    resize.disconnect();
    observer?.disconnect();
    observer = null;
    targets.clear();
    counts.clear();
    track.removeEventListener('pointermove', pointerMove);
    track.removeEventListener('pointerleave', pointerLeave);
    track.removeEventListener('focusin', focusIn);
    track.removeEventListener('focusout', focusOut);
    track.removeEventListener('keydown', keyDown);
    motion.removeEventListener('change', hide);
  };
}
