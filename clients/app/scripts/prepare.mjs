import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { homedir } from 'node:os';
import { cachedStep, fingerprint, syncTree, writeChanged } from './build-cache.mjs';
import { assembleFrontend } from './frontend-assets.mjs';

export const desktop = fileURLToPath(new URL('..', import.meta.url));
export const root = path.resolve(desktop, '../..');
export const dist = path.join(desktop, 'dist');
export const cache = path.join(desktop, '.build-cache');
const env = () => Object.fromEntries(Object.entries(process.env).filter(([key]) => key !== 'NO_COLOR'));
const versions = new Map();

export function commandOutput(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, env: env(), encoding: 'utf8' });
  if (result.error || result.status !== 0) throw result.error ?? new Error(result.stderr || `${command} failed (${result.status})`);
  return result.stdout.trim();
}

export function run(command, args, cwd = root) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, stdio: 'inherit', env: env() });
    child.once('error', reject);
    child.once('close', code => code === 0 ? resolve() : reject(new Error(`${command} failed (${code})`)));
  });
}

export function buildKey(files, tools = [], mode = '') {
  const compilerEnv = Object.entries(process.env).filter(([key]) => /^(RUSTFLAGS|RUSTUP_TOOLCHAIN|CARGO_HOME|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_|CARGO_TARGET_|CARGO_PROFILE_|TAURI_CONFIG)/.test(key)).sort();
  const toolVersions = tools.map(tool => {
    if (!versions.has(tool)) versions.set(tool, commandOutput(tool, ['--version']));
    return [tool, versions.get(tool)];
  });
  const cargoHome = process.env.CARGO_HOME || path.join(homedir(), '.cargo');
  return fingerprint([...files, path.join(cargoHome, 'config'), path.join(cargoHome, 'config.toml'), path.join(root, 'rust-toolchain'), path.join(root, 'rust-toolchain.toml')], { excludeGenerated: true }) + JSON.stringify([mode, process.platform, process.arch, compilerEnv, toolVersions]);
}

export function nativeBinary(release) {
  const metadata = JSON.parse(commandOutput('cargo', ['metadata', '--offline', '--no-deps', '--format-version', '1'], path.join(desktop, 'src-tauri')));
  return path.join(metadata.target_directory, ...(process.env.CARGO_BUILD_TARGET ? [process.env.CARGO_BUILD_TARGET] : []), release ? 'release' : 'debug', 'proteus-desktop');
}

export async function prepareBackend(release) {
  const mode = release ? 'release' : 'debug';
  const metadata = JSON.parse(commandOutput('cargo', ['metadata', '--offline', '--no-deps', '--format-version', '1']));
  const binaries = ['proteus', 'proteus-reference-module'];
  const buildDir = path.join(metadata.target_directory, ...(process.env.CARGO_BUILD_TARGET ? [process.env.CARGO_BUILD_TARGET] : []), mode);
  const sources = [path.join(root, 'Cargo.toml'), path.join(root, 'Cargo.lock'), path.join(root, '.cargo'), path.join(root, 'crates'), path.join(root, 'modules/reference'), path.join(root, 'configs'), path.join(root, 'examples/configs/proteus.coding.example.toml'), path.join(root, 'examples/configs/proteus.example.toml')];
  const args = ['build', '--locked', ...(release ? ['--release'] : []), '-p', 'proteus-core', '-p', 'proteus-reference-module'];
  await cachedStep(cache, `backend-${mode}`, buildKey(sources, ['rustc'], mode) + JSON.stringify(args), binaries.map(name => path.join(buildDir, name)), () => run('cargo', args));
  const resources = path.join(desktop, 'src-tauri/resources'), bin = path.join(resources, 'bin');
  mkdirSync(bin, { recursive: true });
  for (const name of binaries) {
    const target = path.join(bin, name);
    writeChanged(target, readFileSync(path.join(buildDir, name)));
    chmodSync(target, 0o755);
  }
  for (const name of readdirSync(bin)) if (!binaries.includes(name)) rmSync(path.join(bin, name), { recursive: true, force: true });
  const configs = commandOutput('git', ['ls-files', '-z', '--', 'configs']).split('\0').filter(file => file && existsSync(path.join(root, file)));
  const staging = path.join(cache, 'configs');
  rmSync(staging, { recursive: true, force: true });
  mkdirSync(staging, { recursive: true });
  for (const file of configs) writeChanged(path.join(staging, file.slice('configs/'.length)), readFileSync(path.join(root, file)));
  syncTree(staging, path.join(resources, 'configs'));
}

export async function prepareFrontend(release) {
  const mode = release ? 'release' : 'debug';
  // Rendering dependencies are copied only when their source/lock changes.
  await run('node', ['build.mjs'], path.join(desktop, 'ui/rendering'));
  const sharedRust = [path.join(desktop, 'common/src'), path.join(desktop, 'common/Cargo.toml'), path.join(root, 'crates/proteus-contracts'), path.join(root, 'Cargo.toml'), path.join(root, '.cargo')];
  for (const client of ['ui', 'diagnostics']) {
    const directory = path.join(desktop, client);
    const source = ['src', 'Cargo.toml', 'Cargo.lock', 'Trunk.toml', 'index.html'].map(file => path.join(directory, file));
    const args = ['build', '--locked', ...(release ? ['--release'] : [])];
    await cachedStep(cache, `${client}-wasm-${mode}`, buildKey([...source, ...sharedRust], ['rustc', 'trunk'], mode) + JSON.stringify(args), [path.join(directory, 'dist')], () => run('trunk', args, directory), {
      valid: () => existsSync(path.join(directory, 'dist/index.html')) && readdirSync(path.join(directory, 'dist')).some(name => name.endsWith('.wasm')),
    });
  }
  const assetSources = [path.join(desktop, 'common/assets'), path.join(desktop, 'launcher'), path.join(desktop, 'ui/css'), path.join(desktop, 'ui/ui'), path.join(desktop, 'ui/extensions'), path.join(desktop, 'ui/vendor'), path.join(desktop, 'diagnostics/css'), path.join(desktop, 'diagnostics/graph'), path.join(desktop, 'diagnostics/inspector.css'), path.join(desktop, 'ui/dist'), path.join(desktop, 'diagnostics/dist'), path.join(desktop, 'scripts/frontend-assets.mjs')];
  // Compiled dist/vendor are outputs but their contents must participate here.
  const key = fingerprint(assetSources) + mode;
  return cachedStep(cache, `frontend-assets-${mode}`, key, [dist], async () => {
    const staging = dist + '.next';
    rmSync(staging, { recursive: true, force: true });
    mkdirSync(staging, { recursive: true });
    assembleFrontend(desktop, staging, release);
    syncTree(staging, dist);
    rmSync(staging, { recursive: true, force: true });
  }, { valid: () => existsSync(path.join(dist, 'index.html')) && existsSync(path.join(dist, 'inspector.html')) });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const release = process.argv.includes('--release');
  await prepareBackend(release);
  await prepareFrontend(release);
}
