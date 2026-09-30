// Keep a safety buffer around the viewport, refilling it in batches instead of
// mounting/unmounting one row at every wheel step. Retained islands are separate.
export function transcriptViewport(model, offset, viewport, previous = null, overscan = 900) {
  if (!model.rows.length) return { start: 0, end: 0 };
  const margin = overscan / 2;
  if (previous && previous.start >= 0 && previous.end <= model.rows.length
    && previous.start < previous.end
    && model.prefix(previous.start) <= Math.max(0, offset - margin)
    && model.prefix(previous.end) >= Math.min(model.total, offset + viewport + margin)) return previous;
  return {
    start: model.at(Math.max(0, offset - overscan)),
    end: Math.min(model.rows.length, model.at(offset + viewport + overscan) + 1),
  };
}

// Retained cards form islands; they must not mount every intervening message.
export function transcriptWindow(model, offset, viewport, retained = [], selection = null, overscan = 900,
  bounds = transcriptViewport(model, offset, viewport, null, overscan)) {
  if (!model.rows.length) return [];
  const indices = new Set();
  for (let index = bounds.start; index < bounds.end; index++) indices.add(index);
  for (const index of retained) {
    if (index !== undefined && index >= 0 && index < model.rows.length) indices.add(index);
  }
  // A text selection needs its complete interval, including intermediate rows.
  if (selection && selection.every(index => index !== undefined)) {
    for (let index = Math.min(...selection); index <= Math.max(...selection); index++) indices.add(index);
  }
  let previous = -1;
  return [...indices].sort((a, b) => a - b).map(index => {
    const gap = previous < 0 ? 0 : model.prefix(index) - model.prefix(previous + 1);
    previous = index;
    return { index, gap };
  });
}
