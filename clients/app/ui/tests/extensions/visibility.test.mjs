import test from 'node:test';
import assert from 'node:assert/strict';
import { observeVisibility } from '../../extensions/visibility.js';
import { logicallyVisible } from '../../ui/visibility.js';

test('diagnostic iframe follows hidden host tab across its document boundary', () => {
  const host = { nodeType: 1, hidden: true };
  const frame = { nodeType: 1, parentNode: host };
  const page = { nodeType: 9, defaultView: { frameElement: frame } };
  const panel = { nodeType: 1, parentNode: page, isConnected: true };
  assert.equal(logicallyVisible(panel), false);
  host.hidden = false;
  assert.equal(logicallyVisible(panel), true);
});

test('retained panel starts work only while visible and resumes without remounting', () => {
  const previousDocument = globalThis.document;
  const previousObserver = globalThis.MutationObserver;
  const page = new EventTarget();
  page.nodeType = 9;
  page.isConnected = true;
  page.visibilityState = 'visible';
  globalThis.document = page;
  let observer;
  globalThis.MutationObserver = class {
    constructor(callback) { this.callback = callback; observer = this; }
    observe(element) { this.elements ??= []; this.elements.push(element); }
    disconnect() { this.disconnected = true; }
    emit() { this.callback(); }
  };
  try {
    const element = { nodeType: 1, parentNode: page, hidden: true, isConnected: true };
    const controller = new AbortController();
    const transitions = [];
    observeVisibility(element, visible => transitions.push(visible), controller.signal);
    assert.ok(observer.elements.includes(element));
    element.hidden = false;
    observer.emit();
    page.visibilityState = 'hidden';
    page.dispatchEvent(new Event('visibilitychange'));
    page.visibilityState = 'visible';
    page.dispatchEvent(new Event('visibilitychange'));
    element.hidden = true;
    observer.emit();
    assert.deepEqual(transitions, [false, true, false, true, false]);
    controller.abort();
    assert.equal(observer.disconnected, true);
  } finally {
    globalThis.document = previousDocument;
    globalThis.MutationObserver = previousObserver;
  }
});
