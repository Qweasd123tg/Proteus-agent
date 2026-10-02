import test from 'node:test';
import assert from 'node:assert/strict';

test('native command owns dispatch independent of registration order; capture and disposal isolate it', async () => {
  const values = new Map();
  globalThis.window = new EventTarget();
  globalThis.document = {documentElement:{dataset:{}},querySelector:()=>null};
  globalThis.localStorage = {getItem:key=>values.get(key)??null,setItem:(key,value)=>values.set(key,value)};
  const {registerShortcuts, save, snapshot} = await import('../ui/shortcuts/runtime.js');
  const calls = [];
  const native = registerShortcuts(id => {calls.push('native:'+id);return true;},100);
  const web = registerShortcuts(id => {calls.push('web:'+id);return true;});
  function press(code) {
    const event=new Event('keydown',{cancelable:true});
    Object.assign(event,{code,ctrlKey:true,shiftKey:true});
    window.dispatchEvent(event);
    return event.defaultPrevented;
  }
  assert.equal(press('KeyI'),true);
  assert.deepEqual(calls,['native:inspector']);
  document.documentElement.dataset.shortcutRecording='true';
  press('KeyI');
  assert.equal(calls.length,1);
  delete document.documentElement.dataset.shortcutRecording;
  save({...snapshot().bindings,inspector:'Mod+Shift+KeyJ'});
  assert.equal(press('KeyI'),false);
  assert.equal(press('KeyJ'),true);
  assert.deepEqual(calls,['native:inspector','native:inspector']);
  native();press('KeyJ');
  assert.equal(calls.at(-1),'web:inspector');
  web();assert.equal(press('KeyJ'),false);
});
