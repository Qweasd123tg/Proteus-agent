import test from 'node:test';
import assert from 'node:assert/strict';
import { TranscriptHeights } from '../ui/transcript-heights.js';
import { transcriptWindow } from '../ui/transcript-window.js';

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
