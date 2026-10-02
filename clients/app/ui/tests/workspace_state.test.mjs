import { test } from 'node:test';
import assert from 'node:assert/strict';
import { initialLayout, parseLayout, moveTab, closeTab, mergeGroups } from '../ui/workspace/state.mjs';
test('moving across groups retains unique ownership and closing picks a neighbor',()=>{
  const state=initialLayout();state.groups[0].ids.push('files','usage');state.groups.push({ids:[],active:''});
  moveTab(state,'files',1);moveTab(state,'usage',1,'files');
  assert.deepEqual(state.groups.map(g=>g.ids),[['client:chat'],['usage','files']]);
  closeTab(state,'usage');assert.equal(state.groups[1].active,'files');
  mergeGroups(state);assert.deepEqual(state.groups,[{ids:['client:chat','files'],active:'files'}]);
  assert.deepEqual(parseLayout(JSON.stringify(state)),state);
});
test('rejects duplicate tabs, impossible focus and invalid splitter geometry',()=>{
  for(const patch of [{groups:[{ids:['a','a'],active:'a'}]},{focused:2},{ratio:null},{ratio:2},{groups:[]}])assert.throws(()=>parseLayout(JSON.stringify({...initialLayout(),...patch})));
});
