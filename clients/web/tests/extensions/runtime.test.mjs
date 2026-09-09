import test from 'node:test';
import assert from 'node:assert/strict';
import { createPanelRuntime } from '../../extensions/runtime.js';

function runtime(overrides = {}) {
  return createPanelRuntime({ manifest: { entry: 'independent.js', requires: [] }, root: {}, services: {}, storage: {}, onError: error => { throw error; }, ...overrides });
}

test('autonomous extension mounts without agent, aborts and disposes exactly once', async () => {
  let context, disposed = 0;
  const panel = runtime({ load: async () => ({ mount(value) { context = value; return () => disposed++; } }) });
  await panel.ready;
  assert.deepEqual(context.services, {});
  assert.equal(context.signal.aborted, false);
  panel.stop(); panel.stop();
  assert.equal(context.signal.aborted, true);
  assert.equal(disposed, 1);
});

test('only declared interfaces are supplied, with panel-specific cancellation', async () => {
  let received;
  const services = { read: signal => ({ signal, value: 42 }), unrelated: () => assert.fail('must not acquire unrelated interface') };
  const panel = runtime({ manifest: { entry: 'other.js', requires: ['read'] }, services, load: async () => ({ mount(context) { received = context; } }) });
  await panel.ready;
  assert.deepEqual(Object.keys(received.services), ['read']);
  assert.equal(received.services.read.value, 42);
  assert.equal(received.services.read.signal, received.signal);
  panel.stop();
  assert.equal(received.services.read.signal.aborted, true);
});

test('missing interface fails before loading code while another panel keeps working', async () => {
  const errors = [];
  const bad = runtime({ manifest: { entry: 'missing.js', requires: ['missing'] }, onError: error => errors.push(error.message), load: () => assert.fail('must not load') });
  let liveSignal;
  const good = runtime({ load: async () => ({ mount({ signal }) { liveSignal = signal; } }) });
  await Promise.all([bad.ready, good.ready]);
  assert.match(errors[0], /missing/);
  bad.stop();
  assert.equal(liveSignal.aborted, false);
  good.stop();
});

test('disable during import prevents late mount', async () => {
  let resolve;
  const pending = new Promise(done => { resolve = done; });
  const panel = runtime({ load: () => pending });
  panel.stop();
  resolve({ mount: () => assert.fail('stale mount') });
  await panel.ready;
});

test('disable during async mount aborts immediately and disposes late result', async () => {
  let finish, mounted, disposed = 0;
  const entered = new Promise(resolve => { mounted = resolve; });
  const pending = new Promise(resolve => { finish = resolve; });
  let lifetime;
  const panel = runtime({ load: async () => ({ async mount({ signal }) { lifetime = signal; mounted(); await pending; return () => disposed++; } }) });
  await entered;
  panel.stop();
  assert.equal(lifetime.aborted, true);
  finish(); await panel.ready;
  assert.equal(disposed, 1);
});

test('partial mount failure aborts its subscriptions and reports one error', async () => {
  let aborted = false;
  const errors = [];
  const panel = runtime({ onError: error => errors.push(error.message), load: async () => ({ mount({ signal }) {
    signal.addEventListener('abort', () => { aborted = true; });
    throw new Error('broken panel');
  } }) });
  await panel.ready;
  panel.stop();
  assert.equal(aborted, true);
  assert.deepEqual(errors, ['broken panel']);
});
