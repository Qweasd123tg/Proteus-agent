import test from 'node:test';
import assert from 'node:assert/strict';
import { buildRequest, changes, draftFromSnapshot } from '../../ui/modules/agent/draft.js';
import { describeChanges, revisionDraft } from '../../ui/modules/agent/revisions.js';
import { agentSettings } from '../../ui/modules/agent/store.js';
import { ownerOf, packState, setPack } from '../../ui/modules/agent/packs.js';

const snapshot = () => ({
  addon_settings: {addons: {disabled_skills: [], disabled_mcp_servers: [], plugins: []}, mcp_servers: []},
  writable: true,
  target_path: '/tmp/profile/config.toml',
  active_provider: 'main',
  providers: [{ id: 'main', provider: 'openai', model: 'a', label: 'A', active: true }, { id: 'alt', provider: 'openai', model: 'b', label: 'B', active: false }],
  permission_mode: 'normal',
  permission_modes: ['plan', 'normal', 'auto'],
  active_modules: [{ slot: 'workflow', id: 'loop' }],
  hooks: ['first'],
  hook_modules: [{ id: 'first' }, { id: 'second' }],
  module_config: { workflow: { loop: { max_steps: 8 } }, model: { openai: { timeout_ms: 10 } } },
  tools_enabled: ['shell', 'read_file'],
  tools: [],
  plugins: [],
  slots: [{ id: 'workflow', modules: [{ id: 'loop' }, { id: 'plan' }] }],
  warnings: [],
});

test('saved profile round-trips without inventing parameters for untouched modules', () => {
  const saved = snapshot();
  const draft = draftFromSnapshot(saved);
  assert.equal(changes(saved, draft).size, 0);
  assert.deepEqual(buildRequest(saved, draft), {
    addon_settings: saved.addon_settings,
    modules: { workflow: 'loop' },
    hooks: ['first'],
    module_config: saved.module_config,
    tools_enabled: ['read_file', 'shell'],
    active_provider: 'main',
    permission_mode: 'normal',
  });
});

test('changes name the edited areas; formatting alone and invalid JSON are distinguished', () => {
  const saved = snapshot();
  const draft = draftFromSnapshot(saved);
  draft.texts.workflow.loop = '{"max_steps":8}';
  assert.equal(changes(saved, draft).size, 0, 'reformatted JSON is not a change');
  draft.texts.workflow.plan = '{"depth": 2}';
  draft.provider = 'alt';
  draft.mode = 'plan';
  draft.hooks = ['second', 'first'];
  draft.tools = ['shell'];
  assert.deepEqual([...changes(saved, draft)].sort(), ['hook', 'mode', 'provider', 'tools', 'workflow']);
  assert.deepEqual(buildRequest(saved, draft).module_config.workflow, { loop: { max_steps: 8 }, plan: { depth: 2 } });
  draft.texts.model.openai = '{ broken';
  assert.ok(changes(saved, draft).has('model'));
  assert.throws(() => buildRequest(saved, draft), /model\/openai/);
  draft.texts.model.openai = '[]';
  assert.throws(() => buildRequest(saved, draft), /JSON-объектом/);
});

test('a recorded state becomes a draft that rolls the profile back through the builder', () => {
  const saved = snapshot();
  const state = {
    addon_settings: {addons: {disabled_skills: ['review'], disabled_mcp_servers: [], plugins: []}, mcp_servers: []},
    active_provider: 'main',
    permission_mode: 'auto',
    active_modules: [],
    hooks: [],
    module_config: { workflow: { loop: { max_steps: 4 } } },
    tools_enabled: ['shell'],
  };
  const draft = revisionDraft(saved, state);
  assert.equal(draft.modules.workflow, 'loop', 'the builder cannot clear a slot');
  assert.deepEqual(buildRequest(saved, draft).module_config, {
    workflow: { loop: { max_steps: 4 } },
    model: { openai: {} },
  });
  assert.deepEqual(
    describeChanges(draftFromSnapshot(saved), draft).map(({ key, detail }) => [key, detail]),
    [
      ['workflow', 'параметры: max_steps'],
      ['model', 'параметры: timeout_ms'],
      ['hook', 'first → нет'],
      ['addons', 'skills, MCP или пакеты дополнений'],
      ['tools', '− read_file'],
      ['mode', 'По правилам → Правки без вопросов'],
    ],
  );
  assert.deepEqual(buildRequest(saved,draft).addon_settings,state.addon_settings);
});

test('one shared draft saves through the service and survives a rejected save', async () => {
  const requests = [];
  let reject = false;
  const service = {
    read: async () => snapshot(),
    save: async (request) => {
      requests.push(request);
      if (reject) throw Error('plan failed');
      return { ...snapshot(), permission_mode: request.permission_mode };
    },
  };
  await agentSettings.load(service);
  agentSettings.update((draft) => (draft.mode = 'auto'));
  agentSettings.setError('workflow\u001floop\u001fmax_steps', 'Введите число');
  await agentSettings.save(service);
  assert.equal(requests.length, 0, 'field errors block saving');
  agentSettings.setError('workflow\u001floop\u001fmax_steps');
  reject = true;
  await agentSettings.save(service);
  assert.match(agentSettings.state().feedback.text, /plan failed/);
  assert.equal(agentSettings.state().draft.mode, 'auto', 'rejected save keeps the edits');
  reject = false;
  await agentSettings.save(service);
  assert.equal(requests.at(-1).permission_mode, 'auto');
  assert.equal(agentSettings.state().snapshot.permission_mode, 'auto');
  assert.equal(agentSettings.changes().size, 0);
  agentSettings.update((draft) => draft.hooks.push('second'));
  agentSettings.reset();
  assert.deepEqual(agentSettings.state().draft.hooks, ['first']);
  agentSettings.restore({ ...snapshot(), permission_mode: 'plan' });
  assert.deepEqual([...agentSettings.changes()], ['mode']);
});

test('external refresh rebases unsaved parameters without undoing untouched settings',async()=>{
  const saved=snapshot();
  await agentSettings.load({read:async()=>saved},true);
  agentSettings.update(draft=>draft.texts.workflow.loop='{"max_steps":3}');
  const external={...snapshot(),permission_mode:'plan',hooks:['second']};
  await agentSettings.refresh({read:async()=>external});
  const request=buildRequest(external,agentSettings.state().draft);
  assert.equal(request.module_config.workflow.loop.max_steps,3);
  assert.equal(request.permission_mode,'plan');
  assert.deepEqual(request.hooks,['second']);
  assert.match(agentSettings.state().feedback.text,/несохранённые/);
});

test('a tool pack switches as a group in the shared draft and reports mixed state', async () => {
  const owner = { component_id: 'reference', module_id: 'git_tools' };
  const tools = [
    { name: 'git_status', owner, enabled: true, registered: true },
    { name: 'git_diff', owner, enabled: false, registered: false },
    { name: 'git_managed', owner, enabled: true, registered: true, runtime_managed: true },
    { name: 'shell', owner: null, enabled: true, registered: true },
  ];
  const pack = { id: 'git_tools', tools: ['git_status', 'git_diff', 'git_managed'] };
  const saved = { ...snapshot(), tools_enabled: ['git_status', 'shell'], tools,
    plugins: [{ id: 'reference', command: 'proteus-reference-module', description: null,
      exports: [{ slot: 'tool', id: 'git_tools', active: true, description: null, config_schema: null }], tool_packs: [pack] }] };
  assert.deepEqual(ownerOf(tools[1]), { plugin: 'reference', pack: 'git_tools' });
  assert.equal(ownerOf(tools[3]), null, 'ownership is host-reported, not inferred');
  await agentSettings.load({ read: async () => saved }, true);
  const state = () => packState(pack, tools, agentSettings.state().draft.tools);
  assert.deepEqual(state(), { active: 1, total: 2, state: 'mixed' }, 'runtime-managed tools are outside the switch');
  agentSettings.update((draft) => (draft.tools = setPack(draft.tools, pack, tools, true)));
  assert.equal(state().state, 'on');
  assert.deepEqual(buildRequest(saved, agentSettings.state().draft).tools_enabled, ['git_diff', 'git_status', 'shell']);
  agentSettings.update((draft) => (draft.tools = setPack(draft.tools, pack, tools, false)));
  assert.equal(state().state, 'off');
  assert.deepEqual([...agentSettings.changes()], ['tools'], 'a pack is an ordinary tools_enabled edit');
  assert.deepEqual(buildRequest(saved, agentSettings.state().draft).tools_enabled, ['shell'], 'other packs keep their tools');
  // An external profile change keeps the pending pack edit and its other areas.
  await agentSettings.refresh({ read: async () => ({ ...saved, permission_mode: 'plan' }) });
  const request = buildRequest(saved, agentSettings.state().draft);
  assert.deepEqual(request.tools_enabled, ['shell']);
  assert.equal(request.permission_mode, 'plan');
  agentSettings.reset();
  assert.equal(state().state, 'mixed');
});
