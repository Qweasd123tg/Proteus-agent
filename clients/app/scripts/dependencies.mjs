import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { cachedStep, fingerprint } from './build-cache.mjs';

const app = fileURLToPath(new URL('..', import.meta.url));
const npm = spawnSync('npm', ['--version'], { encoding: 'utf8' });
if (npm.status !== 0) throw new Error(npm.stderr || 'npm недоступен');
for (const [name, directory, required] of [
  ['app', app, ['node_modules/.bin/tauri']],
  ['rendering', path.join(app, 'ui/rendering'), ['node_modules/esbuild/lib/main.js', 'node_modules/mathjax/es5']],
]) {
  const manifest = JSON.parse(readFileSync(path.join(directory, 'package.json'), 'utf8'));
  const deps = Object.fromEntries(['dependencies', 'devDependencies', 'optionalDependencies', 'engines'].map(key => [key, Object.entries(manifest[key] ?? {}).sort()]));
  const packageFiles = Object.keys({ ...manifest.dependencies, ...manifest.devDependencies }).map(name => path.join(directory, 'node_modules', name, 'package.json'));
  const key = fingerprint([path.join(directory, 'package-lock.json')]) + JSON.stringify(deps) + process.version + npm.stdout;
  await cachedStep(path.join(app, '.build-cache'), `npm-${name}`, key, [path.join(directory, 'node_modules/.package-lock.json')], () => {
    const result = spawnSync('npm', ['ci', '--no-audit', '--no-fund', ...(name === 'rendering' ? ['--ignore-scripts'] : [])], { cwd: directory, stdio: 'inherit' });
    if (result.status !== 0) throw new Error(`npm ci: ${name}`);
  }, { valid: () => [...packageFiles, ...required.map(file => path.join(directory, file))].every(file => existsSync(file)) });
}
