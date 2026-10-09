import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { canPreview } from '../../extensions/live-preview.js';
import { loadPreviewServices } from '../../extensions/preview-services.js';

const record = (requires, source = 'package') => ({ source, manifest: {
  preview: { entry: 'demo.js' },
  views: [{ surfaces: ['workspace'], entry: 'panel.js', requires, layout: 'scroll', isolation: 'shadow' }],
} });

test('preview availability belongs to the package, not a host service allowlist', () => {
  assert.equal(canPreview(record(['agent.session.read'])), true);
  assert.equal(canPreview(record(['client.diagnostics'])), true);
  assert.equal(canPreview(record([], 'builtin')), false);
  const absent = record([]); delete absent.manifest.preview;
  assert.equal(canPreview(absent), false);
});

test('each shipped preview owns services matching its actual views and releases them on abort', async () => {
  const base = new URL('../../extensions/catalog.json', import.meta.url);
  const catalog = JSON.parse(await readFile(base));
  for (const item of catalog.panels) {
    const url = new URL(item.url, base), manifest = JSON.parse(await readFile(url));
    if (!manifest.preview) continue;
    const preview = new AbortController(), view = new AbortController();
    try {
      const services = await loadPreviewServices({ entry: new URL(manifest.preview.entry, url).href }, preview.signal);
      for (const descriptor of manifest.views) for (const name of descriptor.requires) assert.equal(typeof services[name], 'function', item.id + ': ' + name);
      for (const [name, factory] of Object.entries(services)) {
        const service = factory(view.signal), snapshot = await service.read?.(name === 'agent.workspace.read' ? 'src/main.rs' : undefined);
        if (service.subscribe) {
          const received = []; const stop = service.subscribe(value => received.push(value));
          assert.ok(received.length); stop();
          assert.ok(snapshot);
        }
      }
      if (services['agent.workspace.read']) {
        const workspace = services['agent.workspace.read'](view.signal);
        assert.ok((await workspace.list('')).entries.some(file => file.name === 'src'));
        assert.equal((await workspace.read('src/main.rs')).kind, 'text');
        await assert.rejects(workspace.list('missing'));
      }
      view.abort(); preview.abort();
      for (const factory of Object.values(services)) {
        const service = factory(view.signal);
        if (service.read) await assert.rejects(async () => service.read(), { name: 'AbortError' });
      }
    } finally { view.abort(); preview.abort(); }
  }
});

test('preview loading rejects bad exports and prevents factories after a canceled import', async () => {
  const active = new AbortController();
  for (const implementation of [{}, { createServices: () => null }, { createServices: () => [] }, { createServices: () => ({ fake: {} }) }]) {
    await assert.rejects(loadPreviewServices({ entry: 'demo.js' }, active.signal, async () => implementation));
  }
  let finish, calls = 0;
  const canceled = new AbortController();
  const pending = loadPreviewServices({ entry: 'demo.js' }, canceled.signal, () => new Promise(resolve => finish = resolve));
  canceled.abort(); finish({ createServices() { calls++; return {}; } });
  await assert.rejects(pending, { name: 'AbortError' }); assert.equal(calls, 0);
  const context = new AbortController(); let supplied;
  const loading = loadPreviewServices({ entry: 'demo.js' }, context.signal, async () => ({ createServices(value) { supplied = value; return new Promise(resolve => finish = resolve); } }));
  await Promise.resolve(); context.abort(); finish({});
  await assert.rejects(loading, { name: 'AbortError' });
  assert.deepEqual(Object.keys(supplied), ['signal']); assert.equal(supplied.signal, context.signal);
});
