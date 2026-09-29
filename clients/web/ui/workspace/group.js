import { node } from "../../extensions/dom.js";
import { icon } from "../../extensions/icons.js";
export function button(label, glyph, action, signal) {
  const b = node("button");
  b.type = "button";
  b.title = label;
  b.setAttribute("aria-label", label);
  b.append(icon(glyph));
  b.addEventListener("click", action, { signal });
  return b;
}
export function createGroup(index, { pick, transfer, focus, signal }) {
  const element = node("section", null, "workspace-group");
  element.dataset.group = index;
  element.setAttribute("aria-label", `Область ${index + 1}`);
  const header = node("div", null, "workspace-tabbar"),
    tabs = node("div", null, "workspace-tabs");
  tabs.dataset.group = index;
  tabs.setAttribute("role", "tablist");
  tabs.setAttribute("aria-label", `Вкладки области ${index + 1}`);
  const add = button("Открыть вкладку", "plus", () => pick(index, add), signal);
  add.className = "workspace-add";
  const move = button(
    "Перенести вкладку в соседнюю область",
    "arrow-right",
    () => transfer(index),
    signal,
  );
  move.className = "workspace-transfer";
  const empty = node("div", null, "workspace-empty");
  header.append(tabs, add, move);
  element.append(header, empty);
  for (const type of ["pointerdown", "focusin"])
    element.addEventListener(type, () => focus(index), { signal });
  return { element, tabs, empty, add, move };
}
export function createTab(record) {
  const tab = node("div", null, "workspace-tab");
  tab.dataset.tabId = record.id;
  tab.dataset.owned = String(!!record.owned);
  tab.dataset.client = String(!!record.client);
  const name = node("button", null, "workspace-tab-name");
  name.type = "button";
  name.setAttribute("role", "tab");
  name.id = `workspace-tab-${record.id}`;
  name.setAttribute("aria-controls", `workspace-view-${record.id}`);
  const close = node("button", null, "workspace-tab-close");
  close.type = "button";
  close.append(icon("close"));
  tab.append(name, close);
  return tab;
}
