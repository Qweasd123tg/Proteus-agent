// Recorded profile states compared as drafts, so history uses the same
// change areas as the save bar.
import { diffDrafts, draftFromSnapshot, moduleText, parseParameters } from "./draft.js";
import { changeLabel, permissionText } from "./labels.js";

/**
 * Draft of a recorded state. The builder cannot clear a slot selection, so a
 * slot the state did not select keeps its current module. Parameters the
 * state did not have are cleared explicitly: a save replaces only the
 * modules it names.
 */
export function revisionDraft(snapshot, state) {
  const draft = draftFromSnapshot({ ...snapshot, ...state });
  draft.modules = { ...draftFromSnapshot(snapshot).modules, ...draft.modules };
  for (const [slot, modules] of Object.entries(snapshot.module_config ?? {}))
    for (const module of Object.keys(modules))
      if (!Object.hasOwn(state.module_config?.[slot] ?? {}, module)) {
        draft.texts[slot] ??= {};
        draft.texts[slot][module] = "{}";
      }
  return draft;
}

const sameJson = (left, right) => JSON.stringify(left) === JSON.stringify(right);
const mode = (value) => permissionText[value]?.[0] ?? (value || "—");
const items = (list) => (list.length ? list.join(", ") : "нет");

function parameterNames(before, after, slot) {
  const names = new Set();
  const modules = new Set([
    ...Object.keys(before.texts[slot] ?? {}),
    ...Object.keys(after.texts[slot] ?? {}),
  ]);
  for (const module of modules) {
    const was = parseParameters(moduleText(before, slot, module)).value ?? {};
    const now = parseParameters(moduleText(after, slot, module)).value ?? {};
    for (const name of new Set([...Object.keys(was), ...Object.keys(now)]))
      if (!sameJson(was[name], now[name])) names.add(name);
  }
  return [...names];
}

/** Changed areas with a short before → after summary. */
export function describeChanges(before, after) {
  return [...diffDrafts(before, after)].map((key) => {
    const parts = [];
    if (key === "mode") parts.push(`${mode(before.mode)} → ${mode(after.mode)}`);
    else if (key === "provider") parts.push(`${before.provider || "—"} → ${after.provider || "—"}`);
    else if (key === "tools") {
      const was = new Set(before.tools);
      const now = new Set(after.tools);
      const added = after.tools.filter((name) => !was.has(name));
      const removed = before.tools.filter((name) => !now.has(name));
      if (added.length) parts.push(`+ ${added.join(", ")}`);
      if (removed.length) parts.push(`− ${removed.join(", ")}`);
    } else {
      if (key === "hook" && !sameJson(before.hooks, after.hooks))
        parts.push(`${items(before.hooks)} → ${items(after.hooks)}`);
      if (key !== "hook" && before.modules[key] !== after.modules[key])
        parts.push(`${before.modules[key] ?? "—"} → ${after.modules[key] ?? "—"}`);
      const names = parameterNames(before, after, key);
      if (names.length) parts.push(`параметры: ${names.join(", ")}`);
    }
    return { key, label: changeLabel(key), detail: parts.join("; ") };
  });
}

const time = new Intl.DateTimeFormat("ru", {
  day: "numeric",
  month: "short",
  hour: "2-digit",
  minute: "2-digit",
});
export const revisionTime = (ms) => time.format(new Date(ms));
