// Pure rules for tool packs: a pack is one tool export of a plugin and its
// tools share the profile's `tools_enabled` list. Toggling a pack changes only
// that list; runtime-managed tools stay on and are not part of the switch.

/** Tools of a pack the user can switch, in pack order. */
export function switchable(pack, tools) {
  const managed = new Set(tools.filter((tool) => tool.runtime_managed).map((tool) => tool.name));
  return pack.tools.filter((name) => !managed.has(name));
}

/** `on`, `off` or `mixed` for the switchable tools of a pack in a draft. */
export function packState(pack, tools, enabled) {
  const names = switchable(pack, tools);
  const active = names.filter((name) => enabled.includes(name)).length;
  return {
    active,
    total: names.length,
    state: !names.length || active === names.length ? (active ? "on" : "off") : active ? "mixed" : "off",
  };
}

/** A sorted `tools_enabled` list with every switchable pack tool set to `on`. */
export function setPack(enabled, pack, tools, on) {
  const next = new Set(enabled);
  for (const name of switchable(pack, tools)) on ? next.add(name) : next.delete(name);
  return [...next].sort();
}

/** Owner of a tool as reported by the host; never guessed from its name. */
export function ownerOf(tool) {
  return tool.owner ? { plugin: tool.owner.component_id, pack: tool.owner.module_id } : null;
}
