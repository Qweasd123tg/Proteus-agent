// Export the shipped examples without their source-tree helper dependencies.
import { build } from '../ui/rendering/node_modules/esbuild/lib/main.js';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { parseManifest } from '../ui/extensions/contract.js';

const app = fileURLToPath(new URL('..', import.meta.url));
export async function packageExtension(id, destination) {
  const extensions = path.join(app, 'ui/extensions');
  const catalog = JSON.parse(readFileSync(path.join(extensions, 'catalog.json'), 'utf8'));
  const record = catalog.panels.find(record => record.id === id);
  if (!record) throw Error('Нет поставляемого пакета: ' + id);
  const source = path.dirname(path.resolve(extensions, record.url));
  const manifest = JSON.parse(readFileSync(path.join(source, 'extension.json'), 'utf8'));
  parseManifest(manifest, 'https://package.test/extension.json');
  const output = path.resolve(destination);
  if (existsSync(output)) throw Error('Архив уже существует: ' + output);
  const temporary = mkdtempSync(path.join(existsSync('/tmp/opencode') ? '/tmp/opencode' : tmpdir(), 'proteus-extension-'));
  try {
    mkdirSync(path.join(temporary, 'assets'));
    if (existsSync(path.join(source, 'assets'))) cpSync(path.join(source, 'assets'), path.join(temporary, 'assets'), { recursive: true });
    if (manifest.icon) {
      const asset = path.resolve(source, manifest.icon.src);
      const name = 'icon' + path.extname(asset);
      cpSync(asset, path.join(temporary, 'assets', name));
      manifest.icon.src = './assets/' + name;
    }
    const sprite = readFileSync(path.join(app, 'common/assets/proteus-icons.svg'), 'utf8');
    const drawings = Object.fromEntries([...sprite.matchAll(/<symbol id="([^"]+)"[^>]*>([\s\S]*?)<\/symbol>/g)].map(match => [match[1], match[2]]));
    const entries = manifest.views.map(view => view.entry);
    if (manifest.preview) entries.push(manifest.preview.entry);
    for (const entry of new Set(entries)) {
      if (!/^\.\/[^/]+\.(?:m?js)$/.test(entry)) throw Error('Экспорт примеров требует entry в корне пакета');
      await build({
        entryPoints: [path.resolve(source, entry)], outfile: path.resolve(temporary, entry),
        bundle: true, format: 'esm', platform: 'browser', target: 'es2022',
        plugins: [{ name: 'package-artwork', setup(build) {
          build.onLoad({ filter: /[/\\]extensions[/\\]icons\.js$/ }, args => {
            // External SVG <use> cannot cross the page/package origin boundary.
            const source = readFileSync(args.path, 'utf8');
            const contents = source
              .replace(/const sheet = [^;]+;/, 'const drawings = ' + JSON.stringify(drawings) + ';')
              .replace(/  const use = [\s\S]*?  svg\.append\(use\);/, '  svg.innerHTML = drawings[name] ?? drawings.modules;');
            return { contents, loader: 'js' };
          });
        } }],
      });
    }
    writeFileSync(path.join(temporary, 'extension.json'), JSON.stringify(manifest, null, 2) + '\n');
    const zipped = spawnSync('python3', ['-c', `import pathlib, sys, zipfile
root = pathlib.Path(sys.argv[1])
with zipfile.ZipFile(sys.argv[2], 'x', compression=zipfile.ZIP_DEFLATED) as archive:
    for file in sorted(root.rglob('*')):
        if file.is_file(): archive.write(file, file.relative_to(root))
`, temporary, output], { encoding: 'utf8' });
    if (zipped.status !== 0) throw zipped.error ?? Error(zipped.stderr);
    return output;
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [id, destination] = process.argv.slice(2);
  if (!id || !destination) throw Error('Usage: node clients/app/scripts/package-extension.mjs <id> <output.zip>');
  console.log(await packageExtension(id, destination));
}
