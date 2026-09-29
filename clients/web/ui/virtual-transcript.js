import { TranscriptHeights } from './transcript-heights.js';
import { adjustScroll, requestBottom, cancelBottom } from './transcript-scroll.js';
import { transcriptWindow } from './transcript-window.js';

const controllers = new WeakMap();

export function mountVirtualTranscript(root, onRange, onAdjusted) {
  let model = new TranscriptHeights(), visible = [], frame = 0, measureFrame = 0;
  let windowKey = '';
  let anchor = null, jump = null, stopped = false, width = 0, sessionKey, guardFrame = 0, inputTimer;
  const observed = new Set();
  const top = root.querySelector('[data-transcript-top]');
  const bottom = root.querySelector('[data-transcript-bottom]');
  const pinned = () => root.classList.contains('sticky-bottom');
  const padding = () => parseFloat(getComputedStyle(root).paddingTop) || 0;
  function adjusting() {
    root.dataset.transcriptAdjusting = '';
    cancelAnimationFrame(guardFrame);
  }
  function finishAdjustment() {
    cancelAnimationFrame(guardFrame);
    guardFrame = requestAnimationFrame(() => { guardFrame = 0; delete root.dataset.transcriptAdjusting; });
  }
  function currentAnchor(inset = padding()) {
    if (!model.rows.length) return null;
    const offset = Math.max(0, root.scrollTop - inset);
    const index = model.at(offset);
    return { id: String(model.rows[index].id), offset: offset - model.prefix(index) };
  }
  function restore(value, inset) {
    if (!value) return;
    const index = model.positions.get(value.id);
    if (index !== undefined) adjustScroll(root, inset + model.prefix(index) + value.offset);
  }
  function spacers() {
    top.style.height = `${model.prefix(visible[0]?.index ?? 0)}px`;
    const last = visible.at(-1);
    bottom.style.height = `${Math.max(0, model.total - model.prefix(last ? last.index + 1 : 0))}px`;
  }
  function selectionIndex(node) {
    const element = node?.nodeType === Node.ELEMENT_NODE ? node : node?.parentElement;
    const row = element?.closest('[data-transcript-row]');
    return row && root.contains(row) ? model.positions.get(row.dataset.transcriptRow) : undefined;
  }
  function rangeFor(offset) {
    const viewport = Math.max(1, root.clientHeight);
    const retained = [selectionIndex(document.activeElement)];
    for (const active of root.querySelectorAll('[data-retain-transcript=true]')) retained.push(selectionIndex(active));
    const selection = getSelection();
    const selected = selection && !selection.isCollapsed
      ? [selectionIndex(selection.anchorNode), selectionIndex(selection.focusNode)] : null;
    return transcriptWindow(model, offset, viewport, retained, selected);
  }
  function refresh() {
    frame = 0;
    if (stopped || !root.clientHeight) return;
    const offset = jump !== null ? model.prefix(model.positions.get(jump) ?? 0)
      : pinned() ? Math.max(0, root.scrollTop - padding(), model.total - root.clientHeight)
      : Math.max(0, root.scrollTop - padding());
    const next = rangeFor(offset);
    const key = JSON.stringify(next);
    if (key !== windowKey) {
      anchor ??= currentAnchor();
      adjusting();
      visible = next; windowKey = key;
      spacers();
      onRange(key);
      scheduleMeasure();
    }
    if (pinned()) requestBottom(root);
  }
  function schedule() {
    if (!frame && !stopped && root.clientHeight) frame = requestAnimationFrame(refresh);
  }
  function measure() {
    measureFrame = 0;
    if (!root.clientHeight || stopped) return;
    // Read the complete geometry batch before changing spacers or scrollTop.
    const inset = padding();
    const saved = anchor || currentAnchor(inset);
    const rows = [...root.querySelectorAll(':scope > [data-transcript-row]')];
    const heights = rows.map(row => row.getBoundingClientRect().height);
    const present = new Set(rows);
    for (const old of observed) if (!present.has(old)) { sizes.unobserve(old); observed.delete(old); }
    let changed = false;
    for (const [index, row] of rows.entries()) {
      if (!observed.has(row)) { observed.add(row); sizes.observe(row); }
      changed = model.set(row.dataset.transcriptRow, heights[index]) || changed;
    }
    if (changed || anchor || jump !== null) {
      adjusting();
      // Existing gap nodes update without replacing their retained card owners.
      let previous = -1;
      for (const row of rows) {
        const index = model.positions.get(row.dataset.transcriptRow);
        if (index === undefined) continue;
        const gap = root.querySelector(`[data-transcript-gap="${row.dataset.transcriptRow}"]`);
        if (gap) gap.style.height = `${previous < 0 ? 0 : model.prefix(index) - model.prefix(previous + 1)}px`;
        previous = index;
      }
      spacers();
      if (jump !== null) {
        const row = rows.find(row => row.dataset.transcriptRow === jump);
        if (row) {
          adjustScroll(root, inset + model.prefix(model.positions.get(jump)));
          jump = null;
        }
      } else if (pinned()) requestBottom(root);
      else restore(saved, inset);
      anchor = null;
      schedule();
    }
    finishAdjustment();
  }
  function scheduleMeasure() {
    if (!measureFrame && !stopped) measureFrame = requestAnimationFrame(measure);
  }
  const sizes = new ResizeObserver(scheduleMeasure);
  const rootSize = new ResizeObserver(() => {
    if (!root.clientHeight) return;
    const next = root.clientWidth;
    // Heights measured at the previous width remain estimates until those rows
    // are visited again. Preserve the current anchor while visible rows resize.
    if (next !== width) { anchor ??= currentAnchor(); width = next; scheduleMeasure(); }
    schedule();
  });
  rootSize.observe(root);
  const mutations = new MutationObserver(scheduleMeasure);
  mutations.observe(root, { childList: true });
  const adjusted = event => onAdjusted(Math.round(event.detail));
  const scroll = () => { if (!root.hasAttribute('data-transcript-adjusting')) schedule(); };
  const userInput = event => {
    delete root.dataset.transcriptAdjusting; cancelAnimationFrame(guardFrame);
    // A new gesture takes precedence over the anchor saved by an older mount.
    anchor = null;
    root.dataset.transcriptUserScroll = '';
    if (event?.type === 'pointerdown') delete root.dataset.transcriptDirection;
    clearTimeout(inputTimer);
    inputTimer = setTimeout(() => {
      delete root.dataset.transcriptUserScroll;
      delete root.dataset.transcriptDirection;
    }, 800);
  };
  const keyboardInput = event => {
    if (['ArrowUp','ArrowDown','PageUp','PageDown','Home','End',' '].includes(event.key)
      && (root.contains(document.activeElement) || root.matches(':hover'))
      && !document.activeElement?.matches('input,textarea,[contenteditable=true]')) {
        root.dataset.transcriptDirection = ['ArrowUp','PageUp','Home'].includes(event.key) || (event.key === ' ' && event.shiftKey) ? 'up' : 'down';
        userInput(event);
      }
  };
  const visibility = new MutationObserver(schedule);
  visibility.observe(root, { attributes: true, attributeFilter: ['class'] });
  root.addEventListener('scroll', scroll, { passive: true });
  root.addEventListener('proteus-scroll-adjust', adjusted);
  root.addEventListener('wheel', userInput, { passive: true });
  root.addEventListener('pointerdown', userInput);
  document.addEventListener('keydown', keyboardInput, true);
  document.addEventListener('selectionchange', schedule);
  const controller = {
    update(rows, session) {
      anchor = currentAnchor();
      if (sessionKey !== session) { model = new TranscriptHeights(); anchor = null; jump = null; sessionKey = session; }
      model.reset(rows);
      if (jump !== null && !model.positions.has(jump)) jump = null;
      root.dataset.transcriptCount = String(rows.length);
      // This is a new transcript if its old anchor no longer exists.
      if (anchor && !model.positions.has(anchor.id)) anchor = null;
      windowKey = '';
      refresh();
    },
    jump(id) {
      if (!model.positions.has(String(id))) return false;
      jump = String(id); anchor = null;
      refresh();
      // A target already mounted needs no child-list mutation.
      scheduleMeasure();
      return true;
    },
    dispose() {
      stopped = true;
      cancelAnimationFrame(frame); cancelAnimationFrame(measureFrame); cancelAnimationFrame(guardFrame); cancelBottom(root);
      mutations.disconnect(); sizes.disconnect(); rootSize.disconnect(); visibility.disconnect();
      root.removeEventListener('scroll', scroll);
      root.removeEventListener('proteus-scroll-adjust', adjusted);
      root.removeEventListener('wheel', userInput);
      root.removeEventListener('pointerdown', userInput);
      document.removeEventListener('keydown', keyboardInput, true);
      clearTimeout(inputTimer);
      document.removeEventListener('selectionchange', schedule);
      controllers.delete(root);
    },
  };
  controllers.set(root, controller);
  return () => controller.dispose();
}
export function updateVirtualTranscript(root, rows, session) { controllers.get(root)?.update(JSON.parse(rows), session); }
export function jumpToTranscriptMessage(root, id) { return controllers.get(root)?.jump(id) || false; }
