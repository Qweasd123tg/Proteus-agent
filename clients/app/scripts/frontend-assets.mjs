import { createHash } from 'node:crypto';
import { cpSync, existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const links = html => [...html.matchAll(/<link\b[^>]*>/g)].map(match => ({ tag: match[0], attrs: Object.fromEntries([...match[0].matchAll(/([\w-]+)="([^"]*)"/g)].map(m => [m[1], m[2]])) }));

export function assembleFrontend(app, output, release) {
  const assets = new Map();
  function stylesheet(file, visiting = new Set()) {
    if (assets.has(file)) return assets.get(file);
    if (visiting.has(file)) throw new Error(`Цикл CSS imports: ${file}`);
    visiting = new Set([...visiting, file]);
    const source = readFileSync(file, 'utf8');
    const css = source.replace(/url\(\s*(['"]?)([^)'"\s]+)\1\s*\)|@import\s+(['"])([^'"]+)\3/g, (match, quote, url, importQuote, imported) => {
      const value = url ?? imported;
      if (/^(?:[a-z][a-z\d+.-]*:|\/|#)/i.test(value)) return match;
      const split = value.search(/[?#]/);
      const pathname = split < 0 ? value : value.slice(0, split), suffix = split < 0 ? '' : value.slice(split);
      const target = path.resolve(path.dirname(file), decodeURIComponent(pathname));
      if (!existsSync(target)) throw new Error(`CSS asset отсутствует: ${target}`);
      const name = target.endsWith('.css') ? stylesheet(target, visiting) : asset(target);
      return url !== undefined ? `url("/${name}${suffix}")` : `@import "/${name}${suffix}"`;
    });
    return asset(file, Buffer.from(css));
  }
  function asset(file, bytes = readFileSync(file)) {
    const extension = path.extname(file);
    const hash = createHash('sha256').update(bytes).digest('hex').slice(0, 16);
    const name = `${path.basename(file, extension)}-${hash}${extension}`;
    writeFileSync(path.join(output, name), bytes);
    assets.set(file, name);
    return name;
  }
  for (const client of ['diagnostics', 'ui']) {
    const directory = path.join(app, client), compiled = path.join(directory, 'dist');
    const sourceLinks = links(readFileSync(path.join(directory, 'index.html'), 'utf8'));
    let html = readFileSync(path.join(compiled, 'index.html'), 'utf8');
    const styles = sourceLinks.filter(link => link.attrs['data-trunk'] !== undefined || link.tag.includes('data-trunk')).filter(link => link.attrs.rel === 'css');
    const compiledStyles = links(html).filter(link => link.attrs.rel === 'stylesheet');
    if (styles.length !== compiledStyles.length) throw new Error(`Не совпадает список CSS в ${client}`);
    const copies = sourceLinks.filter(link => link.attrs.rel === 'copy-dir');
    const copiedNames = new Set(copies.map(link => path.basename(link.attrs.href)));
    // Preserve Trunk's WASM loader and snippets, but take static directories from their current sources.
    for (const name of readdirSync(compiled)) {
      if (name === 'index.html' || name.endsWith('.css') || copiedNames.has(name)) continue;
      cpSync(path.join(compiled, name), path.join(output, name), { recursive: true });
    }
    for (const link of copies) cpSync(path.resolve(directory, link.attrs.href), path.join(output, path.basename(link.attrs.href)), { recursive: true });
    for (let i = 0; i < styles.length; i++) {
      const name = stylesheet(path.resolve(directory, styles[i].attrs.href));
      const integrity = createHash('sha384').update(readFileSync(path.join(output, name))).digest('base64');
      html = html.replace(compiledStyles[i].tag, `<link rel="stylesheet" href="/${name}" integrity="sha384-${integrity}"/>`);
    }
    if (!release) html = html.replace('</body>', '<script>new EventSource("/__reload").onmessage = () => location.reload();</script></body>');
    writeFileSync(path.join(output, client === 'ui' ? 'index.html' : 'inspector.html'), html);
  }
  cpSync(path.join(app, 'launcher'), output, { recursive: true });
}
