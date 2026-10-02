import path from 'node:path';
import { cachedStep, fingerprint } from './build-cache.mjs';
import { buildKey, cache, desktop, dist, nativeBinary, prepareBackend, prepareFrontend, root, run } from './prepare.mjs';
import { packagePortable } from './portable.mjs';

await prepareBackend(true);
await prepareFrontend(true);
const binary = nativeBinary(true);
const source = [path.join(desktop, 'src-tauri/src'), path.join(desktop, 'src-tauri/build.rs'), path.join(desktop, 'src-tauri/Cargo.toml'), path.join(desktop, 'src-tauri/Cargo.lock'), path.join(desktop, 'src-tauri/tauri.conf.json'), path.join(desktop, 'src-tauri/icons'), path.join(desktop, 'src-tauri/capabilities'), path.join(desktop, 'common/src'), path.join(desktop, 'common/Cargo.toml'), path.join(root, 'crates/proteus-contracts'), path.join(root, 'Cargo.toml'), path.join(root, '.cargo'), path.join(desktop, 'package-lock.json')];
const args = ['build', '--no-bundle', '--', '--locked'];
const key = buildKey(source, ['rustc'], 'release') + fingerprint([dist]) + JSON.stringify(args);
await cachedStep(cache, 'native-release', key, [binary], () => run(path.join(desktop, 'node_modules/.bin/tauri'), args, desktop));
await packagePortable(binary);
console.log(`Готово: ${path.join(desktop, 'build/Proteus/proteus-desktop')}`);
