import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmodSync, cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cachedStep, fingerprint, syncTree } from './build-cache.mjs';

test('failed stages retry; damaged output is repaired; unchanged copies keep their mtime', async t => {
  const temporary = mkdtempSync(path.join(tmpdir(), 'proteus-cache-'));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const output = path.join(temporary, 'output'), cache = path.join(temporary, 'cache');
  await assert.rejects(cachedStep(cache, 'stage', 'key', [output], () => { throw Error('failed'); }));
  assert.equal(existsSync(path.join(cache, 'stage.json')), false);
  let calls = 0;
  const build = () => { calls++; writeFileSync(output, 'valid'); };
  assert.equal(await cachedStep(cache, 'stage', 'key', [output], build), true);
  assert.equal(await cachedStep(cache, 'stage', 'key', [output], build), false);
  writeFileSync(output, 'corrupt');
  assert.equal(await cachedStep(cache, 'stage', 'key', [output], build), true);
  assert.equal(calls, 2);
  const source = path.join(temporary, 'source'), target = path.join(temporary, 'target');
  mkdirSync(source); writeFileSync(path.join(source, 'file'), 'bytes');
  syncTree(source, target);
  const before = statSync(path.join(target, 'file')).mtimeMs;
  syncTree(source, target);
  assert.equal(statSync(path.join(target, 'file')).mtimeMs, before);
  rmSync(path.join(source, 'file'));
  syncTree(source, target);
  assert.equal(existsSync(path.join(target, 'file')), false);
});

test('one build command skips unchanged stages and isolates CSS, backend and WASM changes', t => {
  const temporary = mkdtempSync(path.join(tmpdir(), 'proteus-build-'));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const app = path.join(temporary, 'clients/app'), mock = path.join(temporary, 'mock');
  function put(file, value = '') { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, value); }
  cpSync(fileURLToPath(new URL('.', import.meta.url)), path.join(app, 'scripts'), { recursive: true });
  cpSync(fileURLToPath(new URL('../../../scripts/desktop.sh', import.meta.url)), path.join(temporary, 'scripts/desktop.sh'), { recursive: true });
  put(path.join(app, 'package.json'), '{"devDependencies":{}}');
  put(path.join(app, 'package-lock.json'), '{}');
  put(path.join(app, 'src-tauri/tauri.conf.json'), '{"productName":"Proteus"}');
  put(path.join(temporary, 'crates/main.rs'), 'backend');
  put(path.join(temporary, 'configs/profile.toml'), 'profile');
  put(path.join(temporary, 'examples/configs/proteus.example.toml'), 'template');
  put(path.join(app, 'launcher/launcher.html'), '<html>launcher</html>');
  put(path.join(app, 'common/assets/logo.svg'), '<svg/>');
  for (const client of ['ui', 'diagnostics']) {
    put(path.join(app, client, 'src/main.rs'), client);
    put(path.join(app, client, 'css/style.css'), '@import "choice.css"; body{background:url("../../common/assets/logo.svg?v=1#logo")}');
    put(path.join(app, client, 'css/choice.css'), 'body{color:red}');
    put(path.join(app, client, 'index.html'), '<html><head><link data-trunk rel="css" href="css/style.css"/><link data-trunk rel="copy-dir" href="../common/assets"/></head><body></body></html>');
  }
  put(path.join(app, 'ui/rendering/package.json'), '{"devDependencies":{}}');
  put(path.join(app, 'ui/rendering/package-lock.json'), '{}');
  put(path.join(app, 'ui/rendering/build.mjs'), `import {mkdirSync,writeFileSync} from 'node:fs'; mkdirSync('../vendor',{recursive:true}); writeFileSync('../vendor/renderer.js','renderer');`);
  const mockProgram = `#!/usr/bin/env node
import {appendFileSync,chmodSync,mkdirSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
const root=process.env.PROTEUS_BUILD_FIXTURE, app=path.join(root,'clients/app');
const command=path.basename(process.argv[1]).replace('.mjs',''), args=process.argv.slice(2);
const put=(file,bytes)=>{mkdirSync(path.dirname(file),{recursive:true});writeFileSync(file,bytes);};
if(args[0]==='--version'){console.log(command+' fixture 1');process.exit(0);}
if(command==='git'){process.stdout.write('configs/profile.toml\\0');process.exit(0);}
if(command==='cargo'&&args[0]==='metadata'){console.log(JSON.stringify({target_directory:path.join(root,process.cwd().endsWith('src-tauri')?'native-target':'target')}));process.exit(0);}
if(command==='npm'&&args[0]==='run'){const result=spawnSync(process.execPath,['scripts/build.mjs'],{cwd:app,env:process.env,stdio:'inherit'});process.exit(result.status??1);}
const phase=command==='trunk'?'trunk-'+path.basename(process.cwd()):command==='npm'?'npm-'+(process.cwd()===app?'app':'rendering'):command;
appendFileSync(path.join(root,'calls'),phase+'\\n');
if(process.env.PROTEUS_FAIL_STAGE===phase)process.exit(3);
if(command==='cargo')for(const name of ['proteus','proteus-reference-module']){const file=path.join(root,'target',args.includes('--release')?'release':'debug',name);put(file,readFileSync(path.join(root,'crates/main.rs')));chmodSync(file,0o755);}
if(command==='trunk'){
 const dir=path.join(process.cwd(),'dist');rmSync(dir,{recursive:true,force:true});
 put(path.join(dir,'index.html'),'<html><head><link rel="stylesheet" href="/old.css"/></head><body><script src="/'+phase+'.js"></script></body></html>');
 put(path.join(dir,phase+'.wasm'),'wasm');put(path.join(dir,phase+'.js'),'loader');put(path.join(dir,'assets/logo.svg'),'stale');
}
if(command==='tauri'){const file=path.join(root,'native-target/release/proteus-desktop');put(file,readFileSync(path.join(app,'dist/index.html')));chmodSync(file,0o755);}
if(command==='npm'){
 put(path.join(process.cwd(),'node_modules/.package-lock.json'),'{}');
 if(process.cwd()===app){const file=path.join(app,'node_modules/.bin/tauri');put(file,readFileSync(path.join(root,'mock/tauri')));chmodSync(file,0o755);}
 else {put(path.join(process.cwd(),'node_modules/esbuild/lib/main.js'),'esbuild');mkdirSync(path.join(process.cwd(),'node_modules/mathjax/es5'),{recursive:true});}
}
`;
  for (const name of ['npm', 'cargo', 'trunk', 'rustc', 'git', 'tauri']) {
    put(path.join(mock, name), mockProgram); chmodSync(path.join(mock, name), 0o755);
  }
  // Extensionless mock executables need an ESM package boundary.
  put(path.join(temporary, 'package.json'), '{"type":"module"}');
  const env = { ...process.env, PATH: mock + path.delimiter + process.env.PATH, PROTEUS_BUILD_FIXTURE: temporary };
  delete env.CARGO_BUILD_TARGET;
  function build(extra = {}) {
    put(path.join(temporary, 'calls'));
    const result = spawnSync('bash', ['scripts/desktop.sh', 'build'], { cwd: temporary, env: { ...env, ...extra }, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    return readFileSync(path.join(temporary, 'calls'), 'utf8').trim().split('\n').filter(Boolean);
  }
  assert.deepEqual(build(), ['npm-app', 'npm-rendering', 'cargo', 'trunk-ui', 'trunk-diagnostics', 'tauri']);
  const bundle = path.join(app, 'build/Proteus');
  assert.equal(readlinkSync(path.join(bundle, 'proteus-desktop')), 'bin/proteus-desktop');
  const before = fingerprint([bundle]);
  assert.deepEqual(build(), []);
  assert.equal(fingerprint([bundle]), before);
  put(path.join(app, 'ui/css/choice.css'), 'body{color:blue}');
  assert.deepEqual(build(), ['tauri']);
  const html = readFileSync(path.join(app, 'dist/index.html'), 'utf8');
  const style = readFileSync(path.join(app, 'dist', html.match(/href="\/([^\"]+)"/)[1]), 'utf8');
  assert.match(html, /integrity="sha384-/);
  assert.match(style, /logo-[a-f0-9]+\.svg\?v=1#logo/);
  assert.equal(readFileSync(path.join(app, 'dist/assets/logo.svg'), 'utf8'), '<svg/>');
  put(path.join(temporary, 'crates/main.rs'), 'changed backend');
  assert.deepEqual(build(), ['cargo']);
  chmodSync(path.join(app, 'src-tauri/resources/bin/proteus'), 0o644);
  assert.deepEqual(build(), []);
  assert.equal(lstatSync(path.join(app, 'src-tauri/resources/bin/proteus')).mode & 0o777, 0o755);
  put(path.join(temporary, 'examples/configs/proteus.example.toml'), 'changed embedded template');
  assert.deepEqual(build(), ['cargo']);
  put(path.join(app, 'ui/src/main.rs'), 'changed ui');
  assert.deepEqual(build(), ['trunk-ui']);
  rmSync(path.join(app, 'ui/dist/trunk-ui.wasm'));
  assert.deepEqual(build(), ['trunk-ui']);
});
