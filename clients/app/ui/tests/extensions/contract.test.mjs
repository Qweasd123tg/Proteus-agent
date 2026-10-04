import test from 'node:test';
import assert from 'node:assert/strict';
import { parseManifest, parseSettings, resourceUrl, hasSurface } from '../../extensions/contract.js';
import { extensionStorage } from '../../extensions/storage.js';

const base = 'https://client.example/extensions/catalog.json';
const manifest = { apiVersion: 1, id: 'test.panel', name: 'Test', description: 'Test', entry: './panel.js', requires: [] };

test('independent package resolves its entry relative to its manifest', () => {
  const parsed = parseManifest(manifest, 'http://localhost:9090/package/extension.json');
  assert.equal(parsed.entry, 'http://localhost:9090/package/panel.js');
  assert.ok(Object.isFrozen(parsed));
  assert.deepEqual(parsed.requires, []);
  assert.equal(parsed.presentation, undefined);
  for (const presentation of ['widget', 'panel']) {
    assert.equal(parseManifest({ ...manifest, presentation }, base).presentation, presentation);
  }
});

test('draft contract rejects unsupported versions, shapes and duplicate interfaces', () => {
  for (const invalid of [null, { ...manifest, apiVersion: 2 }, { ...manifest, backend: {} }, { ...manifest, id: '../a' }, { ...manifest, requires: ['a', 'a'] }, { ...manifest, entry: 'javascript:alert(1)' }, ...[null, '', 'sidebar', 'Panel', false, 1, [], {}].map(presentation => ({ ...manifest, presentation }))]) {
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

test('settings entry resolves independently and rejects malformed capability declarations', () => {
  const parsed = parseManifest({ ...manifest, settings: { entry: './settings.js', requires: [] } }, base);
  assert.equal(parsed.settings.entry, 'https://client.example/extensions/settings.js');
  for (const settings of [null, { entry: './s.js' }, { entry: './s.js', requires: ['a', 'a'] }, { entry: './s.js', requires: [], extra: true }]) assert.throws(() => parseManifest({ ...manifest, settings }, base));
});

test('extension surfaces reject unknown, empty or duplicate values and support compact-only mounting', () => {
  assert.deepEqual(parseManifest(manifest, base).surfaces, ['compact', 'workspace']);
  const compact = parseManifest({...manifest, surfaces:['compact']}, base);
  assert.equal(hasSurface(compact,'workspace'),false);
  assert.equal(hasSurface(compact,'compact'),true);
  assert.ok(Object.isFrozen(compact.surfaces));
  for(const surfaces of [[], ['compact','compact'], ['tabs'], null, 'compact', [false]])assert.throws(()=>parseManifest({...manifest,surfaces},base));
});
