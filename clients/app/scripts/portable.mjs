import { cpSync, mkdirSync, readFileSync, renameSync, rmSync, symlinkSync } from 'node:fs';
import path from 'node:path';
import { desktop } from './prepare.mjs';

// Tauri's PackageInfo.name uses productName, including its case.
const { productName } = JSON.parse(readFileSync(path.join(desktop, 'src-tauri/tauri.conf.json'), 'utf8'));
const destination = path.join(desktop, 'build/Proteus');
const staging = destination + '.next';
rmSync(staging, { recursive: true, force: true });
mkdirSync(path.join(staging, 'bin'), { recursive: true });
cpSync(path.join(desktop, 'src-tauri/target/release/proteus-desktop'), path.join(staging, 'bin/proteus-desktop'));
cpSync(path.join(desktop, 'src-tauri/resources'), path.join(staging, 'lib', productName), { recursive: true });
symlinkSync('bin/proteus-desktop', path.join(staging, 'proteus-desktop'));
rmSync(destination, { recursive: true, force: true });
renameSync(staging, destination);
console.log(`Готово: ${destination}/proteus-desktop`);
