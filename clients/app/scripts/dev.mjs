import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { watch } from 'node:fs';
import path from 'node:path';
import { desktop, dist, root, prepareBackend, prepareFrontend, run } from './prepare.mjs';

await prepareBackend(false);
await prepareFrontend(false);
const listeners = new Set();
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.woff': 'font/woff', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
const server = createServer(async (req, res) => {
  if (req.url === '/__reload') {
    res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
    res.write(': connected\n\n'); listeners.add(res);
    req.on('close', () => listeners.delete(res)); return;
  }
  try {
    const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    let file = path.resolve(dist, '.' + pathname);
    if (!file.startsWith(dist + path.sep) && file !== dist) { res.writeHead(403).end(); return; }
    if (['/', '/resume', '/settings', '/context'].includes(pathname)) file = path.join(dist, 'index.html');
    if (!(await stat(file)).isFile()) { res.writeHead(404).end(); return; }
    res.writeHead(200, { 'Content-Type': mime[path.extname(file)] ?? 'application/octet-stream', 'Cache-Control': 'no-store' });
    res.end(await readFile(file));
  } catch { res.writeHead(404).end(); }
});
server.listen(1430, '127.0.0.1');
let timer, building = false, pending = false;
async function rebuildFrontend() {
  pending = true;
  if (building) return;
  building = true;
  try {
    while (pending) {
      pending = false;
      try {
        await run('node', ['scripts/dependencies.mjs'], desktop);
        if (await prepareFrontend(false)) for (const res of listeners) res.write('data: reload\n\n');
      }
      catch (error) { console.error(error.message); }
    }
  } finally { building = false; }
}
function rebuild() {
  clearTimeout(timer);
  timer = setTimeout(rebuildFrontend, 300);
}
for (const directory of ['clients/app/ui/src', 'clients/app/ui/css', 'clients/app/ui/ui', 'clients/app/ui/extensions', 'clients/app/diagnostics/src', 'clients/app/diagnostics/css', 'clients/app/common/src', 'clients/app/common/assets', 'clients/app/launcher', 'clients/app/diagnostics/graph', 'clients/app/ui/rendering/interactive', 'crates/proteus-contracts/src']) {
  watch(path.join(root, directory), { recursive: true }, rebuild);
}
for (const file of ['clients/app/ui/index.html', 'clients/app/diagnostics/index.html', 'clients/app/diagnostics/inspector.css', 'clients/app/ui/Cargo.toml', 'clients/app/ui/Cargo.lock', 'clients/app/ui/Trunk.toml', 'clients/app/diagnostics/Cargo.toml', 'clients/app/diagnostics/Cargo.lock', 'clients/app/diagnostics/Trunk.toml', 'clients/app/common/Cargo.toml', 'crates/proteus-contracts/Cargo.toml', 'clients/app/ui/rendering/package.json', 'clients/app/ui/rendering/package-lock.json', 'clients/app/ui/rendering/build.mjs']) {
  watch(path.join(root, file), rebuild);
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { for (const res of listeners) res.end(); server.close(); process.exit(0); });
