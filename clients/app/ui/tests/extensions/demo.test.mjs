import test from 'node:test';
import assert from 'node:assert/strict';
import { canPreview } from '../../extensions/live-preview.js';
import { createDemoServices } from '../../extensions/demo-services.js';

const record = (requires, source = 'package', surfaces = ['compact', 'workspace']) =>
  ({ source, manifest: { views: [{ surfaces, entry: 'x.js', requires, layout: 'scroll', isolation: 'shadow' }] } });

test('only packages whose every interface has a demo can be tried', () => {
  assert.equal(canPreview(record(['agent.session.read'])), true);
  assert.equal(canPreview(record([])), true);
  assert.equal(canPreview(record(['client.diagnostics'], 'package', ['settings'])), false);
  assert.equal(canPreview(record(['agent.config.builder'])), false);
  assert.equal(canPreview(record(['client.preferences'], 'builtin', ['settings'])), false);
});

test('demo services keep the real shapes, refuse writes they do not own and stop with the demo', async () => {
  const demo = new AbortController(), view = new AbortController();
  const services = createDemoServices(demo.signal);
  const workspace = services['agent.workspace.read'](view.signal);
  const root = await workspace.list('');
  assert.ok(root.entries.some(entry => entry.path === 'src' && entry.kind === 'directory'));
  assert.equal((await workspace.read('src/main.rs')).kind, 'text');
  await assert.rejects(workspace.list('missing'));
  const composer = services['client.composer'](view.signal);
  assert.throws(() => composer.set('models', []));
  composer.set('effort', 'high');
  assert.equal(composer.read().effortLabel, 'High');
  const snapshots = [];
  services['agent.session.read'](view.signal).subscribe(value => snapshots.push(value));
  assert.equal(snapshots.at(-1).plan.length, 5);
  view.abort(); demo.abort();
  assert.throws(() => composer.read());
});
