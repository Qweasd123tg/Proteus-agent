import test from 'node:test';
import assert from 'node:assert/strict';
import { TranscriptHeights } from '../ui/transcript-heights.js';
import { transcriptViewport, transcriptWindow } from '../ui/transcript-window.js';

test('viewport lookup uses measured heights after resize, append and removal', () => {
  const rows = Array.from({ length: 3000 }, (_, id) => ({ id: String(id), height: 100 }));
  const heights = new TranscriptHeights(rows);
  assert.equal(heights.at(299900), 2999);
  heights.set('20', 100.125);
  assert.equal(heights.prefix(21), 2100.125, 'fractional heights must enter the prefix index');
  const preciseTotal = heights.total;
  heights.reset(rows);
  assert.equal(heights.total, preciseTotal, 'reset must not introduce deferred fractional movement');
  heights.set('20', 180);
  assert.equal(heights.prefix(21), 2180);
  assert.equal(heights.at(2179), 20);
  assert.equal(heights.at(2180), 21);
  heights.reset([...rows, { id: '3000', height: 70 }]);
  assert.equal(heights.total, 300150);
  heights.reset(rows.filter(row => row.id !== '20'));
  assert.equal(heights.total, 299900);
  assert.equal(heights.at(0), 0);
  assert.equal(heights.at(heights.total), 2998);
});

test('retained card islands keep long history bounded and preserve total geometry', () => {
  const rows = Array.from({ length: 3000 }, (_, id) => ({ id: String(id), height: 100 }));
  const heights = new TranscriptHeights(rows);
  heights.set('2', 180);
  heights.set('2998', 230);
  const visible = transcriptWindow(heights, heights.total - 800, 800, [2, 2, undefined]);
  assert.ok(visible.length < 30, 'a retained card must not mount the intervening history');
  assert.equal(visible[0].index, 2);
  assert.equal(visible.at(-1).index, 2999);
  const laidOut = heights.prefix(visible[0].index)
    + visible.reduce((sum, row) => sum + row.gap + heights.values[row.index], 0)
    + heights.total - heights.prefix(visible.at(-1).index + 1);
  assert.equal(laidOut, heights.total, 'sparse retained rows must have the same scroll geometry');
});

test('text selection retains its interval without widening unrelated focused cards', () => {
  const heights = new TranscriptHeights(Array.from({ length: 100 }, (_, id) => ({ id: String(id), height: 100 })));
  const visible = transcriptWindow(heights, 9000, 500, [3], [75, 70], 0);
  const indices = visible.map(row => row.index);
  assert.deepEqual(indices.filter(index => index >= 70 && index <= 75), [70, 71, 72, 73, 74, 75]);
  assert.ok(indices.includes(3));
  assert.ok(!indices.includes(20), 'focus must not retain unrelated intervening rows');
});

test('small wheel steps reuse the viewport buffer without hiding visible rows', () => {
  const heights = new TranscriptHeights(Array.from({ length: 3000 }, (_, id) => ({ id: String(id), height: 100.125 })));
  let previous = null, turnovers = 0;
  for (let step = 0; step < 80; step++) {
    const offset = 12000 + (step < 40 ? -step : step - 80) * 48;
    const bounds = transcriptViewport(heights, offset, 800, previous);
    if (bounds !== previous) turnovers++;
    assert.ok(heights.prefix(bounds.start) <= offset);
    assert.ok(heights.prefix(bounds.end) >= offset + 800);
    previous = bounds;
  }
  assert.ok(turnovers < 12, `wheel steps refilled the buffer ${turnovers} times`);
  const jumped = transcriptViewport(heights, 200000, 800, previous);
  assert.notEqual(jumped, previous, 'a fast gesture must not wait for the old buffer');
  assert.ok(heights.prefix(jumped.start) <= 200000);
  assert.ok(heights.prefix(jumped.end) >= 200800);
  const expanded = transcriptViewport(heights, 200000, 4000, jumped);
  assert.ok(heights.prefix(expanded.end) >= 204000, 'a taller viewport needs immediate coverage');
});

test('viewport buffer handles history boundaries and keeps retained islands independent', () => {
  const heights = new TranscriptHeights(Array.from({ length: 100 }, (_, id) => ({ id: String(id), height: 100 })));
  const head = transcriptViewport(heights, 0, 800);
  assert.equal(head.start, 0);
  assert.equal(transcriptViewport(heights, 48, 800, head), head);
  const tail = transcriptViewport(heights, heights.total - 800, 800, head);
  assert.equal(tail.end, 100);
  assert.equal(transcriptViewport(heights, heights.total - 848, 800, tail), tail);
  const retained = transcriptWindow(heights, 9200, 800, [2], [70, 72], 900, tail);
  assert.ok(retained.some(row => row.index === 2));
  assert.deepEqual(retained.filter(row => row.index >= 70 && row.index <= 72).map(row => row.index), [70, 71, 72]);
  assert.ok(!transcriptWindow(heights, 9200, 800, [], null, 900, tail).some(row => row.index === 2), 'unfocused islands must not enter the viewport buffer');
  heights.reset(heights.rows.slice(0, 5));
  assert.deepEqual(transcriptViewport(heights, 0, 800, tail), { start: 0, end: 5 });
  heights.reset([]);
  assert.deepEqual(transcriptViewport(heights, 0, 800, head), { start: 0, end: 0 });
});
