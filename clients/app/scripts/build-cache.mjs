import { createHash } from 'node:crypto';
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, readlinkSync, readdirSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const generated = new Set(['target', 'node_modules', 'dist', 'vendor', 'build', 'resources', 'gen', '.build-cache', '__pycache__', '.git']);

export function fingerprint(paths, { excludeGenerated = false, accept = () => true } = {}) {
  const hash = createHash('sha256');
  function visit(file) {
    hash.update(file + '\0');
    if (!existsSync(file)) { hash.update('missing\0'); return; }
    const stat = lstatSync(file);
    if (stat.isDirectory()) {
      hash.update('directory\0');
      for (const name of readdirSync(file).sort()) {
        if (excludeGenerated && generated.has(name)) continue;
        const child = path.join(file, name);
        if (lstatSync(child).isDirectory() || accept(child)) visit(child);
      }
    } else {
      hash.update(String(stat.mode & 0o777));
      if (stat.isSymbolicLink()) hash.update(readlinkSync(file));
      hash.update(readFileSync(file));
    }
  }
  for (const file of [...paths].sort()) visit(file);
  return hash.digest('hex');
}

export function writeChanged(file, data) {
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data);
  if (existsSync(file) && readFileSync(file).equals(bytes)) return false;
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file + '.next', bytes);
  renameSync(file + '.next', file);
  return true;
}

export function syncTree(source, destination) {
  mkdirSync(destination, { recursive: true });
  const names = new Set(readdirSync(source));
  for (const name of readdirSync(destination)) {
    if (!names.has(name)) rmSync(path.join(destination, name), { recursive: true, force: true });
  }
  for (const name of names) {
    const from = path.join(source, name), to = path.join(destination, name);
    if (lstatSync(from).isDirectory()) {
      if (existsSync(to) && !lstatSync(to).isDirectory()) rmSync(to, { force: true });
      syncTree(from, to);
    } else if (lstatSync(from).isSymbolicLink()) {
      // Staged bundles have a single executable symlink; preserve its target.
      let same = false;
      try { same = lstatSync(to).isSymbolicLink() && readlinkSync(to) === readlinkSync(from); } catch { /* New entry. */ }
      if (!same) { rmSync(to, { recursive: true, force: true }); symlinkSync(readlinkSync(from), to); }
    } else {
      try { if (!lstatSync(to).isFile()) rmSync(to, { recursive: true, force: true }); } catch { /* New entry. */ }
      writeChanged(to, readFileSync(from));
      const mode = lstatSync(from).mode & 0o777;
      if ((lstatSync(to).mode & 0o777) !== mode) chmodSync(to, mode);
    }
  }
}

export async function cachedStep(cacheDir, name, key, outputs, action, { valid = () => true } = {}) {
  const marker = path.join(cacheDir, name + '.json');
  let previous;
  try { previous = JSON.parse(readFileSync(marker, 'utf8')); } catch { /* A missing disposable cache requires a build. */ }
  const present = () => outputs.every(file => existsSync(file)) && valid();
  let output;
  try { if (present()) output = fingerprint(outputs); } catch { /* Damaged outputs must be rebuilt. */ }
  if (previous?.key === key && output && previous.output === output) return false;
  console.log(`Обновление: ${name}`);
  await action();
  if (!present()) throw new Error(`Стадия ${name} не создала необходимые файлы`);
  writeChanged(marker, JSON.stringify({ key, output: fingerprint(outputs) }) + '\n');
  return true;
}
