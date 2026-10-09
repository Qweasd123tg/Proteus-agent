import { createExtensionRegistry } from '/extensions/registry.js';
import { mountExtensionSettings } from '/extensions/settings.js';
import { createViewRuntime } from '/extensions/runtime.js';
import { extensionIcon } from '/extensions/icons.js';

const invoke = window.__TAURI__.core.invoke;
const assert = (value, message) => { if (!value) throw Error(message); };
const wait = async check => {
  const deadline = performance.now() + 10000;
  while (!check()) {
    if (performance.now() > deadline) throw Error('Timeout: ' + check);
    await new Promise(resolve => setTimeout(resolve, 25));
  }
};
try {
  const registry = createExtensionRegistry();
  await registry.start();
  const stopSettings = mountExtensionSettings(document.querySelector('#settings'), registry);
  if (new URL(location.href).searchParams.get('phase') === 'install') {
    const encoded = await invoke('smoke_fixture');
    const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
    const file = new File([bytes], 'fixture.zip', { type: 'application/zip' });
    const transfer = new DataTransfer();transfer.items.add(file);
    const input = document.querySelector('.extension-install input');
    input.files = transfer.files;
    document.querySelector('.extension-install').requestSubmit();
    await wait(() => !registry.state().busy);
    assert(registry.state().records.some(r => r.id === 'archive-smoke'), registry.state().notice);
    const broken = new File(['broken'], 'broken.zip');
    assert(await registry.install(broken) === false, 'Malformed ZIP was installed');
  }
  const record = registry.state().records.find(r => r.id === 'archive-smoke');
  assert(record?.manifest && record.packageKey, 'Installed record/manifest lost after cold startup: ' + registry.state().notice);
  const picture = extensionIcon(record.manifest);document.body.append(picture);
  await wait(() => picture.complete && picture.naturalWidth > 0);
  const host = document.querySelector('#view'), root = host.attachShadow({mode:'open'});
  let failure;
  const runtime = createViewRuntime({view:record.manifest.views[0],root,services:{},surface:'workspace',onError:error=>failure=error});
  await runtime.ready;
  assert(!failure, 'Native import failed: ' + failure);
  assert(root.textContent.includes('Relative package resource'), 'Relative fetch/import failed');
  await wait(() => getComputedStyle(root.querySelector('p')).color === 'rgb(12, 34, 56)');
  assert(window.smokeMounted === 1, 'Mount count');
  if (new URL(location.href).searchParams.get('phase') === 'cold') {
    const stop = registry.subscribe(() => { if (!registry.state().records.includes(record)) runtime.stop(); });
    document.querySelector('[data-extension-choice=archive-smoke] .extension-actions button').click();
    await wait(() => !registry.state().busy);
    assert(window.smokeAborted === 1 && window.smokeDisposed === 1, 'Remove did not stop lifecycle');
    assert(!registry.state().records.includes(record), 'Removed record still present');
    assert((await fetch(record.url)).status === 404, 'Removed files still served');
    const bytes = Uint8Array.from(atob(await invoke('smoke_fixture')), c => c.charCodeAt(0));
    const transfer = new DataTransfer();transfer.items.add(new File([bytes], 'fixture.zip'));
    document.querySelector('.extension-install').dispatchEvent(new DragEvent('drop', {bubbles:true,cancelable:true,dataTransfer:transfer}));
    await wait(() => !registry.state().busy);
    const dropped = registry.state().records.find(r => r.id === 'archive-smoke');
    assert(dropped?.packageKey && dropped.packageKey !== record.packageKey, 'Drop/reinstall reused old package or failed');
    await registry.remove(dropped.id);
    stop();
  }
  runtime.stop();stopSettings();registry.dispose();
  await invoke('smoke_report', {error:null});
} catch (error) { await invoke('smoke_report', {error:String(error?.stack || error)}); }
