import { TranscriptHeights } from './transcript-heights.js';
import { adjustScroll, requestBottom, cancelBottom } from './transcript-scroll.js';

const controllers = new WeakMap();
const OVERSCAN = 900;

export function mountVirtualTranscript(root, onRange, onAdjusted) {
  let model = new TranscriptHeights(), start = 0, end = 0, frame = 0;
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
  function currentAnchor() {
    if (!model.rows.length) return null;
    const offset = Math.max(0, root.scrollTop - padding());
    const index = model.at(offset);
    return { id: String(model.rows[index].id), offset: offset - model.prefix(index) };
  }
  function restore(value) {
    if (!value) return;
    const index = model.positions.get(value.id);
    if (index !== undefined) adjustScroll(root, padding() + model.prefix(index) + value.offset);
  }
  function spacers() {
    top.style.height = `${model.prefix(start)}px`;
    bottom.style.height = `${Math.max(0, model.total - model.prefix(end))}px`;
  }
  function keep(index, range) {
    if (index === undefined) return;
    range[0] = Math.min(range[0], index);
    range[1] = Math.max(range[1], index + 1);
  }
  function selectionIndex(node) {
    const element = node?.nodeType === Node.ELEMENT_NODE ? node : node?.parentElement;
    const row = element?.closest('[data-transcript-row]');
    return row && root.contains(row) ? model.positions.get(row.dataset.transcriptRow) : undefined;
  }
  function rangeFor(offset) {
    if (!model.rows.length) return [0, 0];
    const viewport = Math.max(1, root.clientHeight);
    const range = [model.at(Math.max(0, offset - OVERSCAN)),
      Math.min(model.rows.length, model.at(offset + viewport + OVERSCAN) + 1)];
    keep(selectionIndex(document.activeElement), range);
    for (const active of root.querySelectorAll('[data-retain-transcript=true]')) keep(selectionIndex(active), range);
    const selection = getSelection();
    if (selection && !selection.isCollapsed) {
      keep(selectionIndex(selection.anchorNode), range);
      keep(selectionIndex(selection.focusNode), range);
    }
    return range;
  }
  function refresh() {
    frame = 0;
    if (stopped || !root.clientHeight) return;
    const offset = jump !== null ? model.prefix(model.positions.get(jump) ?? 0)
      : pinned() ? Math.max(0, root.scrollTop - padding(), model.total - root.clientHeight)
      : Math.max(0, root.scrollTop - padding());
    const next = rangeFor(offset);
    if (next[0] !== start || next[1] !== end) {
      anchor ??= currentAnchor();
      adjusting();
      [start, end] = next;
      spacers();
      onRange(start, end);
      queueMicrotask(measure);
    }
    if (pinned()) requestBottom(root);
  }
  function schedule() {
    if (!frame && !stopped && root.clientHeight) frame = requestAnimationFrame(refresh);
  }
  function measure() {
    if (!root.clientHeight || stopped) return;
    const saved = anchor || currentAnchor();
    const rows = [...root.querySelectorAll(':scope > [data-transcript-row]')];
    const present = new Set(rows);
    for (const old of observed) if (!present.has(old)) { sizes.unobserve(old); observed.delete(old); }
    let changed = false;
    for (const row of rows) {
      if (!observed.has(row)) { observed.add(row); sizes.observe(row); }
      changed = model.set(row.dataset.transcriptRow, row.getBoundingClientRect().height) || changed;
    }
    if (changed || anchor || jump !== null) {
      adjusting();
      spacers();
      if (jump !== null) {
        const row = rows.find(row => row.dataset.transcriptRow === jump);
        if (row) {
          adjustScroll(root, root.scrollTop + row.getBoundingClientRect().top - root.getBoundingClientRect().top);
          jump = null;
        }
      } else if (pinned()) requestBottom(root);
      else restore(saved);
      anchor = null;
      schedule();
    }
    finishAdjustment();
  }
  const sizes = new ResizeObserver(measure);
  const rootSize = new ResizeObserver(() => {
    if (!root.clientHeight) return;
    const next = root.clientWidth;
    // Heights measured at the previous width remain estimates until those rows
    // are visited again. Preserve the current anchor while visible rows resize.
    if (next !== width) { anchor ??= currentAnchor(); width = next; measure(); }
    schedule();
  });
  rootSize.observe(root);
  const mutations = new MutationObserver(measure);
  mutations.observe(root, { childList: true });
  const adjusted = event => onAdjusted(Math.round(event.detail));
  const scroll = () => { if (!root.hasAttribute('data-transcript-adjusting')) schedule(); };
  const userInput = event => {
    delete root.dataset.transcriptAdjusting; cancelAnimationFrame(guardFrame);
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
      start = -1; end = -1;
      refresh();
    },
    jump(id) {
      if (!model.positions.has(String(id))) return false;
      jump = String(id); anchor = null;
      refresh();
      // A target already mounted needs no child-list mutation.
      queueMicrotask(measure);
      return true;
    },
    dispose() {
      stopped = true;
      cancelAnimationFrame(frame); cancelAnimationFrame(guardFrame); cancelBottom(root);
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
