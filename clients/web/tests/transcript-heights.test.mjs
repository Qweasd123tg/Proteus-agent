import test from 'node:test';
import assert from 'node:assert/strict';
import { TranscriptHeights } from '../ui/transcript-heights.js';

test('viewport lookup uses measured heights after resize, append and removal', () => {
  const rows = Array.from({ length: 3000 }, (_, id) => ({ id: String(id), height: 100 }));
  const heights = new TranscriptHeights(rows);
  assert.equal(heights.at(299900), 2999);
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
