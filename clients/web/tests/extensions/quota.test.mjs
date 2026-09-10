import assert from 'node:assert/strict';
import test from 'node:test';
import { duration, remaining, resetLabel } from '../../extensions/model-quota/format.js';

test('quota display retains provider windows and does not infer reset or capacity', () => {
  assert.equal(duration(18000, 'primary'), '5 ч');
  assert.equal(duration(604800, 'secondary'), '7 дн.');
  assert.equal(duration(90, 'rolling'), '90 с');
  assert.equal(duration(null, 'rolling'), 'rolling');
  assert.equal(remaining(105), 0);
  assert.equal(remaining(42.5), 57.5);
  assert.equal(resetLabel(null), 'Время сброса не указано');
  assert.match(resetLabel(100, 101000), /ожидаем новые данные/);
  assert.match(resetLabel(100, 99000), /^Сброс /);
});
