import test from 'node:test';
import assert from 'node:assert/strict';
import { graphModel, layoutGraph, relatedNodes, matches } from '../graph/model.js';

test('map projects server edges, preserves module identity and exposes inactive tools explicitly', () => {
  const snapshot = {
    profile: 'fixture', cwd: '/project', config_files: [], module_epoch: 4, permission_mode: 'ask',
    slots: ['workflow', 'context'].map((id, order) => ({ id, title: id, order, active_module: 'same', category: order ? 'pipeline' : 'orchestrator' })),
    modules: [...['workflow', 'context'].map(slot => ({ id: 'same', slot, active: true, source: { kind: 'process' }, capabilities: [] })),
      { id: 'tool-export', slot: 'tool', active: false, source: { kind: 'process' }, capabilities: [] }],
    tools: [{ name: 'enabled', enabled: true, registered: true }, { name: 'provided', enabled: true, registered: false }],
    edges: [
      { from: 'slot:workflow', to: 'module:workflow:same', kind: 'active_module' },
      { from: 'slot:context', to: 'module:context:same', kind: 'active_module' },
      { from: 'tools', to: 'tool:enabled', kind: 'registered_tool' },
      { from: 'tool:provided', to: 'tools', kind: 'unregistered_tool' },
      { from: 'slot:workflow', to: 'slot:new_contract', kind: 'runtime' },
    ],
  };
  const model = graphModel(snapshot);
  const modules = layoutGraph(model, 'modules');
  assert.equal(modules.nodes.filter(node => node.kind === 'module').length, 2);
  assert.deepEqual(modules.edges, snapshot.edges.slice(0, 2));
  assert.deepEqual([...relatedNodes(model, 'module:context:same')].sort(), ['module:context:same', 'slot:context']);
  assert(layoutGraph(model, 'assembly').nodes.some(node => node.id === 'slot:new_contract'));
  assert(!layoutGraph(model, 'tools').nodes.some(node => node.id === 'tool:provided'));
  assert(layoutGraph(model, 'tools', true).nodes.some(node => node.id === 'tool:provided'));
  assert(layoutGraph(model, 'modules', true).nodes.some(node => node.id === 'module:tool:tool-export'));
  assert(matches(model.nodes.get('module:context:same'), 'CONTEXT'));
  for (const scope of ['assembly', 'modules', 'tools']) {
    const view = layoutGraph(model, scope, true);
    for (const edge of view.edges) assert(snapshot.edges.includes(edge), 'client fabricated an edge');
    assert.equal(new Set(view.nodes.map(node => `${node.x},${node.y}`)).size, view.nodes.length);
  }
});
