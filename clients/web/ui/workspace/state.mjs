export const layoutKey = "proteus.workspace.layout";
export function initialLayout() {
  return {
    groups: [{ ids: ["client:chat"], active: "client:chat" }],
    focused: 0,
    ratio: 0.5,
  };
}
export function parseLayout(raw) {
  if (!raw) return initialLayout();
  const value = JSON.parse(raw),
    seen = new Set();
  if (
    !value ||
    !Array.isArray(value.groups) ||
    value.groups.length < 1 ||
    value.groups.length > 2 ||
    !Number.isFinite(value.ratio) ||
    value.ratio < 0.2 ||
    value.ratio > 0.8 ||
    !Number.isInteger(value.focused) ||
    value.focused < 0 ||
    value.focused >= value.groups.length
  )
    throw Error("Некорректная раскладка вкладок");
  for (const group of value.groups) {
    if (
      !group ||
      !Array.isArray(group.ids) ||
      typeof group.active !== "string" ||
      (group.active && !group.ids.includes(group.active))
    )
      throw Error("Некорректная группа вкладок");
    for (const id of group.ids) {
      if (typeof id !== "string" || !id || seen.has(id))
        throw Error("Повторяющаяся или пустая вкладка");
      seen.add(id);
    }
  }
  return value;
}
export function groupOf(layout, id) {
  return layout.groups.findIndex((g) => g.ids.includes(id));
}
export function moveTab(layout, id, to, before = null) {
  const source = groupOf(layout, id);
  if (source >= 0) {
    const group = layout.groups[source],
      index = group.ids.indexOf(id);
    group.ids.splice(index, 1);
    if (group.active === id)
      group.active =
        (group.ids.includes(group.previous)
          ? group.previous
          : group.ids[Math.min(index, group.ids.length - 1)]) || "";
  }
  const group = layout.groups[to];
  const index = before === null ? -1 : group.ids.indexOf(before);
  group.ids.splice(index < 0 ? group.ids.length : index, 0, id);
  group.previous = group.active;
  group.active = id;
  layout.focused = to;
}
export function closeTab(layout, id) {
  const index = groupOf(layout, id);
  if (index < 0) return;
  const group = layout.groups[index],
    at = group.ids.indexOf(id);
  group.ids.splice(at, 1);
  if (group.active === id)
    group.active =
      (group.ids.includes(group.previous)
        ? group.previous
        : group.ids[Math.min(at, group.ids.length - 1)]) || "";
}
export function mergeGroups(layout) {
  const active = layout.groups[layout.focused].active;
  layout.groups = [{ ids: layout.groups.flatMap((g) => g.ids), active }];
  layout.focused = 0;
}
