import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeArchivePackages } from '../../extensions/archive-packages.js';

test('native ZIP adapter transports file bytes and rolls back canceled installation', async () => {
  const calls = [], controller = new AbortController();
  const installed = { id: 'notes', key: 'key', url: 'proteus-extension://localhost/key/extension.json' };
  const file = { name: 'notes.zip', size: 3, async arrayBuffer() { return Uint8Array.of(0, 128, 255).buffer; } };
  const packages = nativeArchivePackages(async (command, args) => { calls.push([command, args]); return installed; });
  assert.equal(await packages.install(file, ['appearance'], controller.signal), installed);
  assert.deepEqual(calls[0], ['install_ui_extension', { archive: 'AID/', excludedIds: ['appearance'] }]);
  await assert.rejects(packages.install({ ...file, name: 'notes.json' }, [], controller.signal), /ZIP/);
  await assert.rejects(packages.install({ ...file, size: 65 * 1024 * 1024 }, [], controller.signal), /64/);
  const canceled = nativeArchivePackages(async (command, args) => {
    calls.push([command, args]);
    if (command === 'install_ui_extension') controller.abort();
    return installed;
  });
  await assert.rejects(canceled.install(file, [], controller.signal), { name: 'AbortError' });
  assert.deepEqual(calls.at(-1), ['remove_ui_extension', { key: installed.key }]);
  assert.equal(nativeArchivePackages(null).available, false);
  await assert.rejects(nativeArchivePackages(null).install(file, [], new AbortController().signal), /настольном/);
});
