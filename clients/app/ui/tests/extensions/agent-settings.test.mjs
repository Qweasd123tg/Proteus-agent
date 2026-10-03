import test from 'node:test';
import assert from 'node:assert/strict';
import { buildRequest, changes, draftFromSnapshot } from '../../ui/modules/agent/draft.js';
import { describeChanges, revisionDraft } from '../../ui/modules/agent/revisions.js';
import { agentSettings } from '../../ui/modules/agent/store.js';

const snapshot = () => ({
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
  slots: [{ id: 'workflow', modules: [{ id: 'loop' }, { id: 'plan' }] }],
  warnings: [],
});

test('saved profile round-trips without inventing parameters for untouched modules', () => {
  const saved = snapshot();
  const draft = draftFromSnapshot(saved);
  assert.equal(changes(saved, draft).size, 0);
  assert.deepEqual(buildRequest(saved, draft), {
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
      ['tools', '− read_file'],
      ['mode', 'Спрашивать разрешение → Правки без вопросов'],
    ],
  );
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
