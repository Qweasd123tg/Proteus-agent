import test from 'node:test';
import assert from 'node:assert/strict';
import { createViewRuntime } from '../../extensions/runtime.js';

function runtime(overrides = {}) {
  return createViewRuntime({ root: {}, services: {}, storage: {}, surface: 'workspace', onError: error => { throw error; }, ...overrides, view: { surfaces: ['workspace'], entry: 'independent.js', requires: [], ...overrides.view } });
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
  const panel = runtime({ view: { entry: 'other.js', requires: ['read'] }, services, load: async () => ({ mount(context) { received = context; } }) });
  await panel.ready;
  assert.deepEqual(Object.keys(received.services), ['read']);
  assert.equal(received.services.read.value, 42);
  assert.equal(received.services.read.signal, received.signal);
  panel.stop();
  assert.equal(received.services.read.signal.aborted, true);
});

test('missing interface fails before loading code while another panel keeps working', async () => {
  const errors = [];
  const bad = runtime({ view: { entry: 'missing.js', requires: ['missing'] }, onError: error => errors.push(error.message), load: () => assert.fail('must not load') });
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

test('compact-only runtime preserves host hover API and mounts without a panel action', async () => {
  let received;const root={},compact={},hover=Object.freeze({set(){}});
  const panel=runtime({root,compact,hover,surface:'compact',view:{surfaces:['compact']},load:async()=>({mount(context){received=context;}})});
  await panel.ready;assert.equal(received.root,root);assert.equal(received.compact,compact);assert.equal(received.hover,hover);assert.equal(received.panel,undefined);panel.stop();
});

test('a view cannot mount an undeclared surface or acquire another view services', async () => {
  const errors = [];
  const invalid = runtime({ surface: 'settings', onError: error => errors.push(error.message), load: () => assert.fail('undeclared surface must not load') });
  await invalid.ready;
  assert.match(errors[0], /поверхность/);
  invalid.stop();
  const services = { settings: () => ({ value: 1 }), workspace: () => assert.fail('another view service must not be acquired') };
  let received;
  const settings = runtime({ surface: 'settings', view: { surfaces: ['settings'], requires: ['settings'] }, services,
    load: async () => ({ mount(context) { received = context; } }) });
  await settings.ready;
  assert.deepEqual(Object.keys(received.services), ['settings']);
  assert.deepEqual(received.surfaces, ['settings']);
  settings.stop();
});
