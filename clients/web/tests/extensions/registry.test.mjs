import assert from 'node:assert/strict';
import test from 'node:test';
import { createExtensionRegistry } from '../../extensions/registry.js';

function fixture(saved = null) {
  const data = new Map(saved === null ? [] : [['proteus.ui.extensions', saved]]);
  const calls = [];
  const panel = id => ({id,url:`./${id}/extension.json`,enabled:true,collapsed:false});
  const registry = createExtensionRegistry({
    catalogUrl: 'https://client.test/catalog.json',
    storage: {getItem:key=>data.get(key) ?? null,setItem:(key,value)=>data.set(key,value),removeItem:key=>data.delete(key)},
    async readJson(url, signal) {
      calls.push(url); signal.throwIfAborted();
      if(url.endsWith('/catalog.json')) return {url,value:{apiVersion:1,panels:[panel('one'),panel('two')]}};
      const id = new URL(url).pathname.split('/')[1];
      return {url,value:{apiVersion:1,id,name:id,description:id,entry:'./panel.js',requires:[]}};
    },
  });
  return {registry,calls,data};
}

test('shared registry loads once, preserves customization and never reads executable entry points', async () => {
  const saved=JSON.stringify({apiVersion:1,panels:[{id:'two',url:'./two/extension.json',enabled:false,collapsed:true}]});
  const {registry,calls,data}=fixture(saved);
  await Promise.all([registry.start(), registry.start()]);
  assert.equal(calls.length,3);
  assert.equal(registry.state().records[0].enabled,false);
  assert.equal(registry.state().records.length,1);
  assert.ok(calls.every(url=>url.endsWith('.json')));
  let updates=0;const stop=registry.subscribe(()=>updates++);
  registry.addBundled('one'); registry.move('one',-1);
  assert.deepEqual(registry.state().records.map(x=>x.id),['one','two']);
  assert.equal(registry.state().records[1].collapsed,true);
  registry.update('two',{enabled:true});
  assert.equal(JSON.parse(data.get('proteus.ui.extensions')).panels[1].enabled,true);
  stop();const previous=updates;registry.remove('one');assert.equal(updates,previous);
  await registry.start();assert.equal(calls.length,3);
  registry.dispose();
});

test('invalid settings fail explicitly and are replaced only by reset', async () => {
  const {registry,data}=fixture('{"apiVersion":99,"panels":[]}');
  await registry.start();
  assert.equal(registry.state().ready,false);
  assert.match(registry.state().notice,/Не удалось/);
  assert.equal(JSON.parse(data.get('proteus.ui.extensions')).apiVersion,99);
  await registry.reset();
  assert.equal(registry.state().ready,true);
  assert.equal(registry.state().records.length,2);
  assert.equal(JSON.parse(data.get('proteus.ui.extensions')).apiVersion,1);
  registry.dispose();
});
