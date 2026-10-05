import test from 'node:test';
import assert from 'node:assert/strict';
import { readPath, writePath, parseScalar, valueError } from '../../ui/modules/agent/schema.js';

test('editing a nested override and resetting it preserves other and opaque parameters', () => {
  const config = { unknown: { owner: 'kept' } };
  assert.equal(readPath(config, ['capabilities', 'images']), undefined);
  writePath(config, ['capabilities', 'images'], false);
  assert.deepEqual(config, { unknown: { owner: 'kept' }, capabilities: { images: false } });
  writePath(config, ['capabilities', 'images'], undefined);
  assert.deepEqual(config, { unknown: { owner: 'kept' } });
  writePath(config, ['__proto__', 'polluted'], true);
  assert.equal({}.polluted, undefined);
  assert.equal(readPath(config, ['__proto__', 'polluted']), true);
});

test('numbers and choices are validated without guessing or coercing the stored type', () => {
  const integer = { type: 'integer', minimum: 1, maximum: 10 };
  assert.deepEqual(parseScalar(integer, '3'), { value: 3 });
  for (const text of ['', '0', '11', '1.5', 'NaN', '9007199254740993']) assert.ok(parseScalar(integer, text).error, text);
  assert.ok(valueError(integer, '3'));
  const choice = { type: 'enum', options: [{ value: false, title: 'Нет' }, { value: true, title: 'Да' }] };
  assert.equal(valueError(choice, false), null);
  assert.ok(valueError(choice, 'false'));
});
