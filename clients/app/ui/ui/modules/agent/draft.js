// Pure draft rules for the agent profile editor. The server remains the
// authority: it validates the complete assembly before saving the profile.

const pretty = (value) => JSON.stringify(value, null, 2);

export function draftFromSnapshot(snapshot) {
  const texts = {};
  const put = (slot, module, value) => {
    texts[slot] ??= {};
    texts[slot][module] ??= value === undefined ? "{}" : pretty(value);
  };
  for (const [slot, modules] of Object.entries(snapshot.module_config ?? {}))
    for (const [module, value] of Object.entries(modules)) put(slot, module, value);
  for (const slot of snapshot.slots ?? [])
    for (const module of slot.modules) put(slot.id, module.id);
  return {
    modules: Object.fromEntries(
      (snapshot.active_modules ?? []).map((m) => [m.slot, m.id]),
    ),
    hooks: [...(snapshot.hooks ?? [])],
    texts,
    tools: [...(snapshot.tools_enabled ?? [])].sort(),
    provider: snapshot.active_provider ?? "",
    mode: snapshot.permission_mode ?? "",
  };
}

export function moduleText(draft, slot, module) {
  return draft.texts[slot]?.[module] ?? "{}";
}

/** A module's parameters are a JSON object; an empty text means `{}`. */
export function parseParameters(text) {
  if (!text.trim()) return { value: {} };
  try {
    const value = JSON.parse(text);
    if (!value || typeof value !== "object" || Array.isArray(value))
      return { error: "Параметры должны быть JSON-объектом" };
    return { value };
  } catch (error) {
    return { error: error.message };
  }
}

/** Untouched empty parameters of modules absent from the profile are omitted. */
export function moduleConfig(draft, baseline) {
  const result = {};
  for (const [slot, modules] of Object.entries(draft.texts))
    for (const [module, text] of Object.entries(modules)) {
      const parsed = parseParameters(text);
      if (parsed.error) throw Error(`${slot}/${module}: ${parsed.error}`);
      const existed = Object.hasOwn(baseline?.[slot] ?? {}, module);
      if (existed || Object.keys(parsed.value).length) {
        result[slot] ??= {};
        result[slot][module] = parsed.value;
      }
    }
  return result;
}

export function buildRequest(snapshot, draft) {
  return {
    modules: { ...draft.modules },
    hooks: [...draft.hooks],
    module_config: moduleConfig(draft, snapshot.module_config),
    tools_enabled: [...draft.tools],
    active_provider: draft.provider || null,
    permission_mode: draft.mode || null,
  };
}

const sameJson = (left, right) => JSON.stringify(left) === JSON.stringify(right);

/** Areas that differ from the saved profile; invalid JSON counts as a change. */
export function changes(snapshot, draft) {
  return diffDrafts(draftFromSnapshot(snapshot), draft);
}

export function diffDrafts(saved, draft) {
  const result = new Set();
  for (const slot of new Set([
    ...Object.keys(saved.modules),
    ...Object.keys(draft.modules),
  ]))
    if (saved.modules[slot] !== draft.modules[slot]) result.add(slot);
  for (const slot of new Set([
    ...Object.keys(saved.texts),
    ...Object.keys(draft.texts),
  ])) {
    const modules = new Set([
      ...Object.keys(saved.texts[slot] ?? {}),
      ...Object.keys(draft.texts[slot] ?? {}),
    ]);
    for (const module of modules) {
      const before = parseParameters(moduleText(saved, slot, module));
      const after = parseParameters(moduleText(draft, slot, module));
      if (after.error || !sameJson(before.value, after.value)) result.add(slot);
    }
  }
  if (!sameJson(saved.hooks, draft.hooks)) result.add("hook");
  if (!sameJson(saved.tools, [...draft.tools].sort())) result.add("tools");
  if (saved.provider !== draft.provider) result.add("provider");
  if (saved.mode !== draft.mode) result.add("mode");
  return result;
}
