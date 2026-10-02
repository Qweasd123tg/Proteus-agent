import { TranscriptHeights } from './transcript-heights.js';
import { adjustScroll, requestBottom, cancelBottom } from './transcript-scroll.js';
import { transcriptViewport, transcriptWindow } from './transcript-window.js';
import { targetsTranscript } from './transcript-input.js';

const controllers = new WeakMap();

export function mountVirtualTranscript(root, onRange, onAdjusted, onReadingUp) {
  let model = new TranscriptHeights(), visible = [], frame = 0, measureQueued = false;
  let windowKey = '', viewportRange = null;
  let anchor = null, jump = null, jumpAligned = false, stopped = false, width = 0, sessionKey, guardFrame = 0, inputTimer;
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
    guardFrame = requestAnimationFrame(() => {
      guardFrame = 0;
      // Keep the same anchor through the entire pre-paint DOM commit: child
      // effects and ResizeObserver can refine a just-mounted row more than once.
      anchor = null;
      if (jumpAligned) { jump = null; jumpAligned = false; schedule(); }
      delete root.dataset.transcriptAdjusting;
    });
  }
  function currentAnchor(inset = padding()) {
    if (!model.rows.length) return null;
    const offset = Math.max(0, root.scrollTop - inset);
    const index = model.at(offset);
    return { id: String(model.rows[index].id), offset: offset - model.prefix(index) };
  }
  function turnoverAnchor() {
    const value = currentAnchor();
    const row = value && root.querySelector(`[data-transcript-row="${value.id}"]`);
    if (row && root.clientHeight) {
      value.visualTop = row.getBoundingClientRect().top - root.getBoundingClientRect().top;
      value.scrollTop = root.scrollTop;
    }
    return value;
  }
  function restore(value, inset) {
    if (!value) return;
    if (value.visualTop !== undefined) {
      const row = root.querySelector(`[data-transcript-row="${value.id}"]`);
      if (row) {
        // Carry native scrolling between capture and commit into the desired
        // position, so a geometry correction does not undo wheel movement.
        const expected = value.visualTop - (root.scrollTop - value.scrollTop);
        const actual = row.getBoundingClientRect().top - root.getBoundingClientRect().top;
        adjustScroll(root, root.scrollTop + actual - expected);
        value.visualTop = expected;
        value.scrollTop = root.scrollTop;
        return;
      }
    }
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
    viewportRange = transcriptViewport(model, offset, viewport, viewportRange);
    const retained = [selectionIndex(document.activeElement)];
    for (const active of root.querySelectorAll('[data-retain-transcript=true]')) retained.push(selectionIndex(active));
    const selection = getSelection();
    const selected = selection && !selection.isCollapsed
      ? [selectionIndex(selection.anchorNode), selectionIndex(selection.focusNode)] : null;
    return transcriptWindow(model, offset, viewport, retained, selected, 900, viewportRange);
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
      anchor ??= turnoverAnchor();
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
    measureQueued = false;
    if (!root.clientHeight || stopped) return;
    // Read the complete geometry batch before changing spacers or scrollTop.
    const inset = padding();
    const saved = anchor || currentAnchor(inset);
    const rows = [...root.querySelectorAll(':scope > [data-transcript-row]')];
    // Leptos commits the requested window asynchronously. Its child-list
    // observer will retry when those rows exist; never measure the old window
    // against the new spacers or paint one frame with estimated row heights.
    if (rows.length !== visible.length || rows.some((row, index) =>
      row.dataset.transcriptRow !== String(model.rows[visible[index].index]?.id))) return;
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
        if (row && row.getBoundingClientRect().height > 0) {
          // The newly mounted view may still be filling its first contents.
          // Align real DOM geometry instead of consuming an estimated prefix.
          adjustScroll(root, root.scrollTop + row.getBoundingClientRect().top - root.getBoundingClientRect().top);
          jumpAligned = true;
        }
      } else if (pinned()) requestBottom(root);
      else restore(saved, inset);
      schedule();
    }
    finishAdjustment();
  }
  function scheduleMeasure() {
    if (!measureQueued && !stopped) {
      measureQueued = true;
      // Mutation/ResizeObserver work is already batched. Finish geometry before
      // paint instead of exposing the estimated window for an extra frame.
      queueMicrotask(measure);
    }
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
  mutations.observe(root, { childList: true, subtree: true });
  const adjusted = event => onAdjusted(Math.round(event.detail));
  const scroll = () => { if (!root.hasAttribute('data-transcript-adjusting')) schedule(); };
  const userInput = (event, direction) => {
    if (event?.type === 'wheel' && !targetsTranscript(root, event)) return;
    const previousDirection = root.dataset.transcriptDirection;
    if (event?.type === 'wheel') direction = event.deltaY < 0 ? 'up' : 'down';
    if (direction) root.dataset.transcriptDirection = direction;
    // The wheel listener is passive; leave browser scrolling in charge and only
    // cross WASM once when an upward gesture actually leaves follow mode.
    if (direction === 'up' && (previousDirection !== 'up' || pinned())) onReadingUp();
    delete root.dataset.transcriptAdjusting; cancelAnimationFrame(guardFrame);
    // A new gesture takes precedence over the anchor saved by an older mount.
    anchor = null;
    jump = null; jumpAligned = false;
    root.dataset.transcriptUserScroll = '';
    if (event?.type === 'pointerdown') delete root.dataset.transcriptDirection;
    clearTimeout(inputTimer);
    inputTimer = setTimeout(() => {
      delete root.dataset.transcriptUserScroll;
      delete root.dataset.transcriptDirection;
    }, 800);
  };
  const keyboardInput = event => {
    const focused = document.activeElement;
    const documentFocused = !focused || focused === document.body || focused === document.documentElement;
    const direction = ['ArrowUp','PageUp','Home'].includes(event.key) || (event.key === ' ' && event.shiftKey) ? 'up' : 'down';
    if (['ArrowUp','ArrowDown','PageUp','PageDown','Home','End',' '].includes(event.key)
      && (root.contains(focused) || (documentFocused && root.matches(':hover')))
      && !focused?.closest('input,textarea,select,[contenteditable]:not([contenteditable=false])')
      && targetsTranscript(root, event, direction === 'up' ? -1 : 1)) {
        userInput(event, direction);
      }
  };
  const visibility = new MutationObserver(schedule);
  visibility.observe(root, { attributes: true, attributeFilter: ['class'] });
  root.addEventListener('scroll', scroll, { passive: true });
  root.addEventListener('proteus-scroll-adjust', adjusted);
  root.addEventListener('wheel', userInput, { passive: true });
  root.addEventListener('pointerdown', userInput);
  document.addEventListener('keydown', keyboardInput);
  document.addEventListener('selectionchange', schedule);
  const controller = {
    update(rows, session) {
      anchor = turnoverAnchor();
      if (sessionKey !== session) { model = new TranscriptHeights(); anchor = null; jump = null; jumpAligned = false; sessionKey = session; }
      model.reset(rows);
      viewportRange = null;
      if (jump !== null && !model.positions.has(jump)) jump = null;
      root.dataset.transcriptCount = String(rows.length);
      // This is a new transcript if its old anchor no longer exists.
      if (anchor && !model.positions.has(anchor.id)) anchor = null;
      windowKey = '';
      refresh();
    },
    jump(id) {
      if (!model.positions.has(String(id))) return false;
      jump = String(id); jumpAligned = false; anchor = null;
      refresh();
      // A target already mounted needs no child-list mutation.
      scheduleMeasure();
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
      document.removeEventListener('keydown', keyboardInput);
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
