import test from 'node:test';
import assert from 'node:assert/strict';
import {defaults, fromEvent, rebind, validate, label} from '../ui/shortcuts/catalog.mjs';

test('physical bindings handle layouts, exact modifiers, IME and repeats', () => {
  assert.equal(fromEvent({code:'KeyL',key:'д',ctrlKey:true}), 'Mod+KeyL');
  assert.equal(fromEvent({code:'KeyL',metaKey:true},true), 'Mod+KeyL');
  assert.equal(fromEvent({code:'KeyL',ctrlKey:true,shiftKey:true}), 'Mod+Shift+KeyL');
  assert.equal(fromEvent({code:'KeyL',ctrlKey:true,repeat:true}), null);
  assert.equal(fromEvent({code:'KeyL',ctrlKey:true,isComposing:true}), null);
  assert.equal(fromEvent({code:'KeyL',ctrlKey:true,getModifierState:()=>true}), null);
  assert.equal(label('Mod+Comma',true),'Cmd + ,');
});
test('remapping rejects conflicts and editing keys; disable and reset are validated', () => {
  const initial=defaults();
  assert.deepEqual(validate(initial),initial);
  assert.throws(()=>rebind(initial,'settings','Mod+KeyB'),/Уже назначено/);
  for(const binding of ['KeyA','Shift+KeyA','Mod+KeyV','Enter','Mod+Enter','Tab','Alt+F4']) assert.throws(()=>rebind(initial,'settings',binding));
  const changed=rebind(initial,'sidebar',null);
  assert.equal(rebind(changed,'settings','Mod+KeyB').settings,'Mod+KeyB');
  assert.throws(()=>rebind({...changed,settings:'Mod+KeyB'},'sidebar',initial.sidebar),/Уже назначено/);
  assert.throws(()=>validate({...initial,unknown:null}));
  assert.throws(()=>validate({}));
});
