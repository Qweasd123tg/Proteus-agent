// Retained cards form islands; they must not mount every intervening message.
export function transcriptWindow(model, offset, viewport, retained = [], selection = null, overscan = 900) {
  if (!model.rows.length) return [];
  const start = model.at(Math.max(0, offset - overscan));
  const end = Math.min(model.rows.length, model.at(offset + viewport + overscan) + 1);
  const indices = new Set();
  for (let index = start; index < end; index++) indices.add(index);
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
