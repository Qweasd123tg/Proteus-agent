import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, readFileSync, readdirSync, writeFileSync, chmodSync, rmSync, renameSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export const desktop = fileURLToPath(new URL('..', import.meta.url));
export const root = path.resolve(desktop, '../..');
export const dist = path.join(desktop, 'dist');
export function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, stdio: 'inherit', env: Object.fromEntries(Object.entries(process.env).filter(([key]) => key !== 'NO_COLOR')) });
  if (result.error || result.status !== 0) throw result.error ?? new Error(`${command} failed (${result.status})`);
}

export function prepareBackend(release) {
  run('cargo', ['build', '--locked', ...(release ? ['--release'] : []), '-p', 'proteus-core', '-p', 'proteus-reference-worker']);
  const resources = path.join(desktop, 'src-tauri/resources');
  mkdirSync(path.join(resources, 'bin'), { recursive: true });
  for (const name of ['proteus', 'proteus-reference-worker']) {
    const target = path.join(resources, 'bin', name);
    cpSync(path.join(root, 'target', release ? 'release' : 'debug', name), target + '.next');
    chmodSync(target + '.next', 0o755);
    renameSync(target + '.next', target);
  }
  rmSync(path.join(resources, 'configs'), { recursive: true, force: true });
  const tracked = spawnSync('git', ['ls-files', '-z', '--', 'configs'], { cwd: root, encoding: 'utf8' });
  if (tracked.status !== 0) throw new Error('Cannot list packaged configs');
  for (const file of tracked.stdout.split('\0').filter(Boolean)) {
    const target = path.join(resources, file);
    mkdirSync(path.dirname(target), { recursive: true });
    cpSync(path.join(root, file), target);
  }
}

export function prepareFrontend(release) {
  const output = dist + '.next';
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  for (const client of ['web', 'inspector']) {
    run('trunk', ['build', '--locked', ...(release ? ['--release'] : [])], path.join(root, 'clients', client));
    const source = path.join(root, 'clients', client, 'dist');
    for (const item of readdirSync(source)) {
      if (item !== 'index.html') cpSync(path.join(source, item), path.join(output, item), { recursive: true });
    }
    let html = readFileSync(path.join(source, 'index.html'), 'utf8')
      .replace('https://cdn.jsdelivr.net/npm/mathjax@3/es5/', '/vendor/mathjax/')
      .replace('https://cdn.jsdelivr.net/npm/mermaid@11/dist/', '/vendor/mermaid/');
    if (!release) html = html.replace('</body>', '<script>new EventSource("/__reload").onmessage = () => location.reload();</script></body>');
    writeFileSync(path.join(output, client === 'web' ? 'index.html' : 'inspector.html'), html);
  }
  cpSync(path.join(desktop, 'launcher'), output, { recursive: true });
  const vendorOptions = { recursive: true, filter: source => !source.endsWith('.map') && !source.endsWith('.d.ts') };
  cpSync(path.join(desktop, 'node_modules/mathjax/es5'), path.join(output, 'vendor/mathjax'), vendorOptions);
  cpSync(path.join(desktop, 'node_modules/mermaid/dist'), path.join(output, 'vendor/mermaid'), vendorOptions);
  for (const library of ['mathjax', 'mermaid']) {
    cpSync(path.join(desktop, 'node_modules', library, 'LICENSE'), path.join(output, 'vendor', library, 'LICENSE'));
  }
  rmSync(dist + '.previous', { recursive: true, force: true });
  if (existsSync(dist)) renameSync(dist, dist + '.previous');
  renameSync(output, dist);
  rmSync(dist + '.previous', { recursive: true, force: true });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const release = process.argv.includes('--release');
  prepareBackend(release);
  prepareFrontend(release);
}
