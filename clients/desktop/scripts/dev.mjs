import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { watch } from 'node:fs';
import path from 'node:path';
import { dist, root, prepareBackend, prepareFrontend } from './prepare.mjs';

prepareBackend(false);
prepareFrontend(false);
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
let timer;
function rebuild() {
  clearTimeout(timer);
  timer = setTimeout(() => {
    try { prepareFrontend(false); for (const res of listeners) res.write('data: reload\n\n'); }
    catch (error) { console.error(error.message); }
  }, 300);
}
for (const directory of ['clients/web/src', 'clients/web/css', 'clients/web/extensions', 'clients/inspector/src', 'clients/inspector/css', 'clients/common/src', 'clients/desktop/launcher']) {
  watch(path.join(root, directory), { recursive: true }, rebuild);
}
for (const file of ['clients/web/index.html', 'clients/inspector/index.html', 'clients/inspector/inspector.css']) {
  watch(path.join(root, file), rebuild);
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { for (const res of listeners) res.end(); server.close(); process.exit(0); });
