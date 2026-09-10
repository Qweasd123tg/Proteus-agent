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
