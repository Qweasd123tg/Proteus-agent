import { cpSync, mkdirSync, readFileSync, rmSync, symlinkSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { cachedStep, fingerprint, syncTree } from './build-cache.mjs';
import { cache, desktop, nativeBinary } from './prepare.mjs';

export async function packagePortable(binary = nativeBinary(true)) {
  const { productName } = JSON.parse(readFileSync(path.join(desktop, 'src-tauri/tauri.conf.json'), 'utf8'));
  const resources = path.join(desktop, 'src-tauri/resources');
  const destination = path.join(desktop, 'build/Proteus');
  const key = fingerprint([binary, resources, path.join(desktop, 'src-tauri/tauri.conf.json'), fileURLToPath(import.meta.url)]);
  return cachedStep(cache, 'portable-release', key, [destination], async () => {
    const staging = destination + '.next';
    rmSync(staging, { recursive: true, force: true });
    mkdirSync(path.join(staging, 'bin'), { recursive: true });
    cpSync(binary, path.join(staging, 'bin/proteus-desktop'));
    cpSync(resources, path.join(staging, 'lib', productName), { recursive: true });
    symlinkSync('bin/proteus-desktop', path.join(staging, 'proteus-desktop'));
    syncTree(staging, destination);
    rmSync(staging, { recursive: true, force: true });
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await packagePortable();
  console.log(`Готово: ${desktop}/build/Proteus/proteus-desktop`);
}
