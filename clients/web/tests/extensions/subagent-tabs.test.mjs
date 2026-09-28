import test from 'node:test';
import assert from 'node:assert/strict';
import { attachClientTabHost, openSubagentTab, closeSubagentTab } from '../../extensions/subagent-tabs.js';

function parent() {
  return { children: [], append(child) {
    if (child.parentNode) child.parentNode.children = child.parentNode.children.filter(item => item !== child);
    this.children.push(child); child.parentNode = this;
  } };
}

test('one workspace-owned tab preserves live content and restores it on close or host disposal', () => {
  const parking = parent(), content = {}, records = [];
  parking.append(content);
  const stop = attachClientTabHost({ create(key, spec) {
    const record = { key, spec, root: parent(), shown: 0, closed: 0 };
    records.push(record);
    return { root: record.root, show() { record.shown++; }, close() { record.closed++; spec.onClose(); } };
  } });
  let released = 0;
  assert.equal(openSubagentTab('subagent-7', 'Субагент · обзор', content, () => released++), true);
  assert.equal(content.parentNode, records[0].root);
  assert.equal(openSubagentTab('subagent-7', 'Субагент · обзор', content, () => assert.fail('duplicate callback')), false);
  assert.equal(records.length, 1);
  assert.equal(records[0].shown, 2);
  closeSubagentTab('subagent-7');
  assert.equal(content.parentNode, parking);
  assert.equal(released, 1);
  closeSubagentTab('subagent-7');
  assert.equal(released, 1);
  openSubagentTab('subagent-7', 'Субагент · обзор', content, () => released++);
  stop();
  assert.equal(content.parentNode, parking);
  assert.equal(released, 2);
  assert.equal(records[1].closed, 1);
});

test('a view with no mounted workspace is released immediately', () => {
  let released = 0;
  const parking = parent(), content = {};
  parking.append(content);
  assert.equal(openSubagentTab('subagent-8', 'Субагент', content, () => released++), false);
  assert.equal(released, 1);
  assert.equal(content.parentNode, parking);
});
