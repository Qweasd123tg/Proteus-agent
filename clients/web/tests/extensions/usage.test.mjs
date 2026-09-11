import test from 'node:test';
import assert from 'node:assert/strict';
import { requestCost, validateRates, BUILTIN_RATES } from '../../extensions/usage/pricing.js';
import { summarize } from '../../extensions/usage/summary.js';

const request = (input = 1000) => ({ model: { provider: 'subscription', model: 'gpt-5.6-luna' }, origin: 'direct', status: 'completed', usage: { input_tokens: input, output_tokens: 200, cached_input_tokens: 800, cache_creation_input_tokens: 100, reasoning_output_tokens: 80 } });

test('cost partitions cached input and output reasoning without double counting', () => {
  const item = request();
  assert.ok(Math.abs(requestCost(item, []).value - .000301) < 1e-12);
  const total = summarize([item, { ...item, origin: 'compactor' }], []);
  assert.equal(total.input + total.output, 2400);
  assert.equal(total.reasoning, 160);
  assert.equal(total.compactions, 1);
  assert.equal(total.reported, 2);
});

test('long-context premium is applied per request and only above the threshold', () => {
  const short = request(272000), long = request(272001);
  assert.equal(requestCost(short, []).long, false);
  assert.equal(requestCost(long, []).long, true);
  const expected = requestCost(short, []).value + requestCost(long, []).value;
  assert.equal(summarize([short, long], []).cost, expected);
});

test('unknown rates, failed requests and partial provider data never become a zero bill', () => {
  const missing = { ...request(), usage: null, status: 'error' };
  const unknown = { ...request(), model: { provider: 'other', model: 'gpt-5.6-luna-invented' } };
  assert.equal(requestCost(missing, []).value, null);
  assert.equal(requestCost(unknown, []).value, null);
  const total = summarize([request(), missing, unknown], []);
  assert.equal(total.requests, 3); assert.equal(total.reported, 2); assert.equal(total.priced, 1); assert.equal(total.errors, 1);
  const partial = summarize([request(), { ...request(), usage: { input_tokens: 100, output_tokens: 20 } }], []);
  assert.equal(partial.detailsReported.cached, 1);
  assert.equal(partial.detailsReported.reasoning, 1);
  assert.equal(requestCost(request(10), []).value, null, 'inconsistent cache categories must not produce a price');
});

test('explicit provider rates override model-wide rates and invalid tables fail', () => {
  const rate = { ...BUILTIN_RATES[4], input: 1, output: 1, cached: 1, write: 1, threshold: 0 };
  const specific = { ...rate, provider: 'subscription', input: 2, output: 2, cached: 2, write: 2 };
  const rates = validateRates([rate, specific]);
  assert.ok(Math.abs(requestCost(request(), rates).value - .0024) < 1e-12);
  assert.throws(() => validateRates([rate, rate]));
  assert.throws(() => validateRates([{ ...rate, input: -1 }]));
  assert.throws(() => validateRates([{ ...rate, output: Infinity }]));
  assert.throws(() => validateRates([{ ...rate, wrong: 1 }]));
});

const { scopedRequests, selectRequests, requestExport } = await import('../../extensions/usage/selection.js');
const exchange = (id, extra = {}) => ({ ...request(), exchange_id: id, turn_id: 'turn-a', started_at_ms: 1000, finished_at_ms: 1100, ...extra });

test('diagnostic filters preserve scope totals and keep compaction and unassigned requests addressable', () => {
  const items = [exchange('ok'), exchange('failed', { status: 'error', usage: null }),
    exchange('summary', { origin: 'compactor', turn_id: 'turn-b' }), exchange('outside', { turn_id: null, status: 'unfinished', finished_at_ms: null })];
  const snapshot = { latest_turn_id: 'turn-b', requests: items };
  const scoped = scopedRequests(snapshot, 'all', 'all');
  assert.deepEqual(selectRequests(scoped, { status: 'problems' }, []).map(item => item.exchange_id), ['failed', 'outside']);
  assert.deepEqual(selectRequests(scoped, { status: 'missing_usage' }, []).map(item => item.exchange_id), ['failed']);
  assert.deepEqual(selectRequests(scoped, { query: 'SUMMARY turn-B', origin: 'compactor' }, []).map(item => item.exchange_id), ['summary']);
  assert.deepEqual(scopedRequests(snapshot, 'latest', 'all').map(item => item.exchange_id), ['summary']);
  assert.equal(summarize(scoped, []).requests, 4, 'list filters must not change the scope summary');
  assert.equal(scopedRequests({ ...snapshot, latest_turn_id: null }, 'latest', 'all').length, 0);
});

test('diagnostic sorting is stable and missing duration, usage and pricing sort last', () => {
  const a = exchange('a'), b = exchange('b');
  const unknown = exchange('unknown', { usage: null, finished_at_ms: null });
  const long = exchange('long', { finished_at_ms: 9100, usage: { input_tokens: 5000, output_tokens: 20 } });
  for (const sort of ['duration', 'tokens', 'cost']) {
    assert.deepEqual(selectRequests([unknown, a, b, long], { sort }, []).map(item => item.exchange_id), ['long', 'a', 'b', 'unknown']);
  }
  const later = exchange('later', { started_at_ms: 2000 });
  assert.deepEqual(selectRequests([a, later, b], { sort: 'oldest' }, []).map(item => item.exchange_id), ['a', 'b', 'later']);
  assert.deepEqual(selectRequests([a, later, b], { sort: 'newest' }, []).map(item => item.exchange_id), ['later', 'a', 'b']);
});

test('JSON export includes the entire filtered selection and exact source identifiers', () => {
  const snapshot = { session_id: 'session-1', revision: 7, requests: Array.from({ length: 27 }, (_, i) => exchange(`id-${i}`)) };
  const selected = selectRequests(snapshot.requests, { sort: 'newest' }, []);
  const exported = JSON.parse(requestExport(snapshot, selected, { sort: 'newest' }));
  assert.equal(exported.session_id, 'session-1');
  assert.equal(exported.revision, 7);
  assert.equal(exported.requests.length, 27, 'export must not be limited to the visible page');
  assert.deepEqual(exported.requests, selected);
  assert.deepEqual(exported.filters, { sort: 'newest' });
});
