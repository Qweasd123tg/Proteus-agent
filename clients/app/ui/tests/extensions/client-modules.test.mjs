import test from 'node:test';
import assert from 'node:assert/strict';
import {createExtensionRegistry} from '../../extensions/registry.js';
import {createClientModuleRegistry} from '../../ui/modules/registry.js';
import {builtins} from '../../ui/modules/catalog.js';
import {parseManifest} from '../../extensions/contract.js';
const catalog=builtins.map(r=>({...r,manifest:{...r.manifest,entry:'https://client.test/'+r.id+'.js'}}));
function fixture(data=new Map()){
 const storage={getItem:k=>data.get(k)??null,setItem:(k,v)=>data.set(k,v),removeItem:k=>data.delete(k)};
 const packages=createExtensionRegistry({storage,catalogUrl:'https://client.test/catalog.json',reservedIds:catalog.map(r=>r.id),readJson:async url=>({url,value:url.endsWith('catalog.json')?{apiVersion:1,panels:[]}:{apiVersion:1,id:url.includes('collision')?'appearance':'custom',name:'Custom',description:'Diagnostic/model replacement',entry:'./module.js',requires:['client.composer'],surfaces:['settings','composer-model'],navigation:{group:'diagnostics',icon:'analysis'}}})});
 const registry=createClientModuleRegistry(packages,storage,catalog);return {registry,storage,data};
}
test('installed settings/model module uses the same selection contract and survives restart',async()=>{
 const {registry,data}=fixture();await registry.start();
 assert.equal(registry.selected('composer-model').id,'model-selector');
 await registry.install('https://client.test/custom.json');assert.ok(registry.select('composer-model','custom'));
 assert.equal(registry.selected('composer-model').id,'custom');
 registry.update('custom',{enabled:false});assert.equal(registry.selected('composer-model'),undefined);
 registry.update('custom',{enabled:true});assert.equal(registry.selected('composer-model').id,'custom');
 const reload=fixture(data).registry;await reload.start();assert.equal(reload.selected('composer-model').id,'custom');
 reload.remove('custom');assert.equal(reload.selected('composer-model'),undefined);
 reload.resetCore();assert.equal(reload.selected('composer-model').id,'model-selector');
});
test('management cannot be disabled or removed; invalid slots and write failures retain configuration',()=>{
 const {registry,storage}=fixture();registry.update('extensions',{enabled:false});registry.remove('extensions');
 assert.ok(registry.state().records.find(r=>r.id==='extensions').enabled);
 assert.equal(registry.select('composer-model','access-selector'),false);
 storage.setItem=()=>{throw Error('disk');};registry.update('model-selector',{enabled:false});
 assert.equal(registry.selected('composer-model').id,'model-selector');assert.match(registry.state().notice,/сохранить/);
});
test('corrupt config reports an error with management available; reserved ids cannot collide',async()=>{
 const {registry}=fixture(new Map([['proteus.ui.modules','{"slots":{}}']]));
 assert.match(registry.state().notice,/Неверный формат/);assert.equal(registry.selected('composer-model'),undefined);
 assert.ok(registry.state().records.find(r=>r.id==='extensions').enabled);registry.resetCore();
 await registry.start();assert.equal(await registry.install('https://client.test/collision.json'),false);
 assert.equal(registry.state().records.filter(r=>r.id==='appearance').length,1);
});
test('only user choices are stored, so built-in pages added or removed by an update keep the selection',()=>{
 const {registry,data}=fixture();registry.update('chat',{enabled:false});
 assert.deepEqual(JSON.parse(data.get('proteus.ui.modules')),{disabled:['chat'],slots:{'composer-model':'model-selector','composer-access':'access-selector'}});
 const shorter=catalog.filter(r=>r.id!=='diagnostic-usage');
 const fewer=createClientModuleRegistry(createExtensionRegistry({storage:{getItem:k=>data.get(k)??null,setItem:(k,v)=>data.set(k,v),removeItem:k=>data.delete(k)},catalogUrl:'https://client.test/catalog.json',reservedIds:shorter.map(r=>r.id),readJson:async()=>({})}),undefined,shorter);
 assert.equal(fewer.state().notice,'');assert.equal(fewer.state().records.find(r=>r.id==='chat').enabled,false);
 assert.ok(fewer.state().records.find(r=>r.id==='appearance').enabled,'a page missing from storage is on');
 for(const [stored,message] of [
  ['{"disabled":["removed-page"],"slots":{"composer-model":null,"composer-access":null}}',/«removed-page» больше нет/],
  ['{"disabled":["extensions"],"slots":{"composer-model":null,"composer-access":null}}',/нельзя выключить/],
  ['{"enabled":{"chat":true},"slots":{"composer-model":null,"composer-access":null}}',/Неверный формат/],
 ]){
  const broken=fixture(new Map([['proteus.ui.modules',stored]])).registry;
  assert.match(broken.state().notice,message);assert.equal(broken.state().builtinsInvalid,true);
  broken.resetCore();assert.equal(broken.state().builtinsInvalid,false);assert.equal(broken.state().notice,'');
 }
});
test('navigation only belongs to settings; unknown surfaces and fields fail explicitly',()=>{
 const value={apiVersion:1,id:'fixture',name:'Fixture',description:'',entry:'./module.js',requires:[],surfaces:['settings'],navigation:{group:'diagnostics',icon:'analysis'}};
 assert.deepEqual(parseManifest(value,'https://client.test/fixture.json').surfaces,['settings']);
 for(const change of [{surfaces:['unknown']},{surfaces:['workspace']},{navigation:{group:'made-up',icon:'analysis'}},{navigation:{group:'settings',icon:'analysis',extra:true}}])assert.throws(()=>parseManifest({...value,...change},'https://client.test/fixture.json'));
});

test('retained module services rebind sessions and reject late results from the old session',async()=>{
 const {createAgentServices}=await import('../../extensions/agent-services.js');
 const bridge=createAgentServices(),signal=new AbortController().signal;
 const service=bridge.services['agent.config.read'](signal);
 let connected=false;
 const startup=service.read().then(value=>{connected=true;return value});
 await Promise.resolve();assert.equal(connected,false);
 const canceled=new AbortController(),reason=new Error('module closed');
 const abandoned=bridge.services['agent.config.read'](canceled.signal).read();
 canceled.abort(reason);await assert.rejects(abandoned,error=>error===reason);
 const firstRelease=bridge.bind({readConfig:async()=>'{"session":"first"}'});
 assert.deepEqual(await startup,{session:'first'});
 firstRelease();
 const reconnect=service.read();
 const reconnectRelease=bridge.bind({readConfig:async()=>'{"session":"reconnected"}'});
 assert.deepEqual(await reconnect,{session:'reconnected'});
 reconnectRelease();
 let finish;const release=bridge.bind({readConfig:()=>new Promise(resolve=>finish=resolve)});
 const pending=service.read();bridge.bind({readConfig:async()=>'{"session":"new"}'});
 finish('{"session":"old"}');await assert.rejects(pending,/изменилась/);
 release();assert.deepEqual(await service.read(),{session:'new'});
});

test('a stored package cannot duplicate a builtin record',async()=>{
 const saved=JSON.stringify({apiVersion:1,panels:[{id:'appearance',url:'https://client.test/collision.json',enabled:true,collapsed:false}]});
 const {registry}=fixture(new Map([['proteus.ui.extensions',saved]]));await registry.start();
 assert.equal(registry.state().records.filter(r=>r.id==='appearance').length,1);
 assert.match(registry.state().notice,/идентификатор встроенного/);
});
