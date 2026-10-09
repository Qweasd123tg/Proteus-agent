import assert from 'node:assert/strict';
import test from 'node:test';
import { createExtensionRegistry } from '../../extensions/registry.js';

function fixture(saved = null, options = {}) {
  const data = new Map(saved === null ? [] : [['proteus.ui.extensions', saved]]);
  const calls = [];
  const removed = [];
  const panel = id => ({id,url:`./${id}/extension.json`,enabled:true,collapsed:false});
  const registry = createExtensionRegistry({
    catalogUrl: 'https://client.test/catalog.json',
    storage: {getItem:key=>data.get(key) ?? null,setItem:(key,value)=>data.set(key,value),removeItem:key=>data.delete(key)},
    packages: { available: true, async install(file) { return { id: file.id, key: '11111111-1111-4111-8111-111111111111', url: `https://client.test/${file.id}/extension.json` }; }, async remove(key) { removed.push(key); } },
    async readJson(url, signal) {
      calls.push(url); signal.throwIfAborted();
      if(url.endsWith('/catalog.json')) return {url,value:{apiVersion:1,panels:[panel('one'),panel('two')]}};
      const id = new URL(url).pathname.split('/')[1];
      return {url,value:{apiVersion:3,id,name:id,description:id,views:[{surfaces:['compact','workspace'],entry:'./panel.js',requires:[],layout:'scroll',isolation:'shadow'}]}};
    },
    ...options,
  });
  return {registry,calls,data,removed};
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
  registry.update('two',{enabled:true,location:'left'});
  assert.equal(registry.state().records[1].location,'left');
  assert.equal(JSON.parse(data.get('proteus.ui.extensions')).panels[1].location,'left');
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

// A pointer drop may cross several rows; their relative order must survive.
test('moving across several positions inserts without swapping other rows', async () => {
  const {registry,data}=fixture();await registry.start();
  await registry.install({id:'three'});
  registry.move('three',-2);
  assert.deepEqual(registry.state().records.map(x=>x.id),['three','one','two']);
  assert.deepEqual(JSON.parse(data.get('proteus.ui.extensions')).panels.map(x=>x.id),['three','one','two']);
  registry.move('three',2);
  assert.deepEqual(registry.state().records.map(x=>x.id),['one','two','three']);
  registry.dispose();
});

test('ZIP records survive a cold registry; remove and reset release files after publishing the list', async () => {
  const {registry,data,removed}=fixture();await registry.start();
  assert.equal(await registry.install({id:'three'}),true);
  const key=registry.state().records.at(-1).packageKey;
  registry.update('three',{enabled:false});registry.dispose();
  const cold=fixture(data.get('proteus.ui.extensions'));
  await cold.registry.start();
  assert.equal(cold.registry.state().records.at(-1).packageKey,key);
  assert.equal(cold.registry.state().records.at(-1).enabled,false);
  await cold.registry.remove('three');assert.deepEqual(cold.removed,[key]);
  assert.equal(cold.registry.state().records.some(r=>r.id==='three'),false);
  const restored=fixture(data.get('proteus.ui.extensions'));
  await restored.registry.start();await restored.registry.reset();
  assert.deepEqual(restored.removed,[key]);
  assert.deepEqual(restored.registry.state().records.map(r=>r.id),['one','two']);
  assert.deepEqual(removed,[]);
});

test('failed install persistence rolls back files; failed removal persistence keeps the record', async () => {
  const {registry,removed}=fixture(null,{storage:{getItem:()=>null,setItem(){throw Error('disk full')}}});
  await registry.start();
  assert.equal(await registry.install({id:'three'}),false);
  assert.equal(registry.state().records.length,2);assert.equal(removed.length,1);
  assert.match(registry.state().notice,/disk full/);
  await registry.remove('one');assert.equal(registry.state().records.length,2);
  assert.match(registry.state().notice,/осталось/);
});
