import test from 'node:test';
import assert from 'node:assert/strict';
import { parseManifest, parseSettings, resourceUrl, hasSurface, viewForSurface } from '../../extensions/contract.js';
import { extensionStorage } from '../../extensions/storage.js';
import { readFile } from 'node:fs/promises';

const base = 'https://client.example/extensions/catalog.json';
const view = { surfaces: ['compact', 'workspace'], entry: './panel.js', requires: [], layout: 'scroll', isolation: 'shadow' };
const manifest = { apiVersion: 2, id: 'test.panel', name: 'Test', description: 'Test', views: [view] };

test('all shipped packages conform to the same view contract as builtins', async () => {
  const { builtins } = await import('../../ui/modules/catalog.js');
  for (const record of builtins) {
    assert.equal(record.source, 'builtin');
    const manifest = { ...record.manifest, views: record.manifest.views.map(view => ({ ...view, entry: 'https://client.test/' + new URL(view.entry).pathname.split('/').pop() + new URL(view.entry).search })) };
    parseManifest(manifest, 'https://client.test/catalog.js');
  }
  const catalogUrl = new URL('../../extensions/catalog.json', import.meta.url);
  const catalog = JSON.parse(await readFile(catalogUrl, 'utf8'));
  for (const record of catalog.panels) {
    const url = new URL(record.url, catalogUrl);
    const value = JSON.parse(await readFile(url, 'utf8'));
    const parsed = parseManifest(value, 'https://client.test/' + record.id + '/extension.json');
    assert.equal(parsed.id, record.id);
    assert.equal(builtins.some(builtin => builtin.id === record.id), false);
  }
});

test('independent package resolves its entry relative to its manifest', () => {
  const parsed = parseManifest(manifest, 'http://localhost:9090/package/extension.json');
  assert.equal(parsed.views[0].entry, 'http://localhost:9090/package/panel.js');
  assert.ok(Object.isFrozen(parsed));
  assert.deepEqual(parsed.views[0].requires, []);
  assert.equal(viewForSurface(parsed, 'compact'), viewForSurface(parsed, 'workspace'));
  for (const layout of ['scroll', 'fill', 'form', 'editor']) {
    assert.equal(parseManifest({ ...manifest, views: [{ ...view, layout }] }, base).views[0].layout, layout);
  }
});

test('draft contract rejects unsupported versions, shapes and duplicate interfaces', () => {
  for (const invalid of [null, { ...manifest, apiVersion: 1 }, { ...manifest, backend: {} }, { ...manifest, id: '../a' }, ...[
    { requires: ['a', 'a'] }, { entry: 'javascript:alert(1)' }, { layout: 'diagnostics' }, { isolation: 'unknown' }, { navigation: {} }, { requires: null },
  ].map(change => ({ ...manifest, views: [{ ...view, ...change }] })), ...['entry','requires','settings','presentation','surfaces','navigation'].map(field => ({ ...manifest, [field]: manifest[field] }))]) {
    assert.throws(() => parseManifest(invalid, base));
  }
  assert.throws(() => resourceUrl('https://user:password@example.com/plugin.json', base));
});

test('settings preserve explicit order and reject malformed or duplicate panels', () => {
  const panel = { id: 'test', url: './test/extension.json', enabled: false, collapsed: true };
  const settings = { apiVersion: 1, panels: [panel, { ...panel, id: 'next', enabled: true }] };
  const parsed = parseSettings(settings, base);
  assert.equal(parsed[0].location, 'right');
  for (const location of ['left', 'right']) {
    assert.equal(parseSettings({ ...settings, panels: [{ ...panel, location }] }, base)[0].location, location);
  }
  for (const location of ['main', 'floating']) {
    assert.throws(() => parseSettings({ ...settings, panels: [{ ...panel, location }] }, base));
  }
  assert.deepEqual(parsed.map(({ id, enabled }) => [id, enabled]), [['test', false], ['next', true]]);
  assert.equal(parsed[0].url, 'https://client.example/extensions/test/extension.json');
  for (const invalid of [{ ...settings, panels: [panel, panel] }, { ...settings, apiVersion: 0 }, { ...settings, panels: [{ ...panel, enabled: 'yes' }] }]) {
    assert.throws(() => parseSettings(invalid, base));
  }
});

test('two extensions keep independent local data without an agent', () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const a = extensionStorage(storage, 'a');
  const b = extensionStorage(storage, 'b');
  let updates = 0;
  const stop = extensionStorage(storage, 'a').subscribe(() => updates++);
  a.set('text', 'first'); b.set('text', 'second'); a.remove('text');
  assert.equal(a.get('text'), null);
  assert.equal(b.get('text'), 'second');
  assert.equal(updates, 2);
  stop(); a.set('text', 'after disposal'); assert.equal(updates, 2);
});

test('storage events invalidate matching document subscribers and detach on disposal', () => {
  const target = new EventTarget();
  const originalAdd = globalThis.addEventListener, originalRemove = globalThis.removeEventListener;
  let attached = 0;
  globalThis.addEventListener = (...args) => { attached++; target.addEventListener(...args); };
  globalThis.removeEventListener = (...args) => { attached--; target.removeEventListener(...args); };
  const storage = { getItem:()=>null, setItem(){}, removeItem(){} };
  let usage = 0, notes = 0;
  const stopUsage = extensionStorage(storage, 'usage').subscribe(()=>usage++);
  const stopNotes = extensionStorage(storage, 'notes').subscribe(()=>notes++);
  function emit(key, area=storage){
    const event = new Event('storage');
    Object.defineProperties(event, {key:{value:key}, storageArea:{value:area}});
    target.dispatchEvent(event);
  }
  try {
    assert.equal(attached, 1);
    emit('proteus.ui.extension.usage:pricing');
    assert.deepEqual([usage,notes], [1,0]);
    emit('other'); emit('proteus.ui.extension.usage:pricing', {});
    assert.deepEqual([usage,notes], [1,0]);
    emit(null);
    assert.deepEqual([usage,notes], [2,1]);
    stopUsage(); emit('proteus.ui.extension.usage:pricing');
    assert.deepEqual([usage,notes], [2,1]);
    stopNotes(); assert.equal(attached,0);
  } finally {
    stopUsage(); stopNotes();
    if(originalAdd) globalThis.addEventListener = originalAdd; else delete globalThis.addEventListener;
    if(originalRemove) globalThis.removeEventListener = originalRemove; else delete globalThis.removeEventListener;
  }
});

test('each view resolves independently; settings declare their own services and layout', () => {
  const parsed = parseManifest({ ...manifest, views: [view, { ...view, surfaces: ['settings'], entry: './settings.js', requires: ['config.read'], layout: 'form' }] }, base);
  const settings = viewForSurface(parsed, 'settings');
  assert.equal(settings.entry, 'https://client.example/extensions/settings.js');
  assert.deepEqual(settings.requires, ['config.read']);
  assert.deepEqual(viewForSurface(parsed, 'workspace').requires, []);
  assert.ok(Object.isFrozen(settings) && Object.isFrozen(settings.surfaces));
  assert.throws(() => parseManifest({ ...manifest, views: [view, { ...view, surfaces: ['workspace'] }] }, base));
});

test('views reject unknown, empty or duplicate surfaces and explicit old shapes', () => {
  const compact = parseManifest({...manifest, views:[{...view,surfaces:['compact']}]}, base);
  assert.equal(hasSurface(compact,'workspace'),false);
  assert.equal(hasSurface(compact,'compact'),true);
  assert.ok(Object.isFrozen(compact.views));
  for(const surfaces of [[], ['compact','compact'], ['tabs'], null, 'compact', [false], ['settings','composer-model']])assert.throws(()=>parseManifest({...manifest,views:[{...view,surfaces}]},base));
  for(const views of [[],null,'views'])assert.throws(()=>parseManifest({...manifest,views},base));
});
