import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { packageExtension } from './package-extension.mjs';

test('every shipped ZIP contains its manifests, artwork and bundled JavaScript helpers', async t => {
  const temporary = mkdtempSync(path.join(tmpdir(), 'proteus-extension-export-'));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const catalog = JSON.parse(readFileSync(new URL('../ui/extensions/catalog.json', import.meta.url)));
  for (const { id } of catalog.panels) {
    const file = await packageExtension(id, path.join(temporary, id + '.zip'));
    const checked = spawnSync('python3', ['-c', `import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    manifest = json.loads(archive.read('extension.json'))
    assert manifest['apiVersion'] == 3
    for field in ('icon', 'preview'):
        assert archive.read(manifest[field]['src'].removeprefix('./'))
    for view in manifest['views']:
        source = archive.read(view['entry'].removeprefix('./')).decode()
        assert "from '../" not in source and "from './" not in source
        assert 'proteus-icons.svg' not in source and 'svg.append(use)' not in source
`, file], { encoding: 'utf8' });
    assert.equal(checked.status, 0, checked.stderr);
  }
  await assert.rejects(packageExtension('notes', path.join(temporary, 'notes.zip')), /существует/);
});
