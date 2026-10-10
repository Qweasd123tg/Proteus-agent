import { agentSettings } from "./store.js";
import { button, el } from "./page.js";
import { parametersEditor } from "./editor.js";
import { openAgentTarget } from "./focus.js";
import { packState, setPack } from "./packs.js";
import { safetyText, slotPage, slotText } from "./labels.js";

const title = (slot) => slotText[slot]?.[0] ?? slot;
const css = (value) => CSS.escape(value);

/** Module parameters open on demand; one editor per export and page build. */
function parameters(row, slot, id, view) {
  const details = el("details", "agent-hook-parameters");
  details.append(el("summary", "", "Параметры"));
  row.append(details);
  let editor;
  details.addEventListener("toggle", () => {
    if (!details.open || editor) return;
    editor = parametersEditor(details, slot, id, view.signal);
    editor.sync(agentSettings.state());
  }, { signal: view.signal });
  view.sync((state) => editor?.sync(state));
}

function check(label, onChange, view) {
  const input = el("input", "settings-toggle");
  input.type = "checkbox";
  input.setAttribute("aria-label", label);
  input.addEventListener("change", () => onChange(input.checked), { signal: view.signal });
  return input;
}

/** A tool export: one switch for the pack and one for each of its tools. */
function packBlock(plugin, exported, snapshot, view) {
  const pack = plugin.tool_packs.find((item) => item.id === exported.id) ?? { id: exported.id, tools: [] };
  const tools = snapshot.tools;
  const block = el("section", "agent-pack");
  block.dataset.agentPack = pack.id;
  const head = el("div", "agent-hook-head");
  const text = el("span", "agent-choice-text");
  const name = el("strong", "", pack.id);
  const count = el("span", "agent-count");
  name.append(count);
  text.append(name, el("span", "settings-hint", exported.description?.trim() || "Пакет инструментов"));
  const group = check(`Все инструменты пакета ${pack.id}`, (on) =>
    agentSettings.update((draft) => (draft.tools = setPack(draft.tools, pack, tools, on))), view);
  group.dataset.agentPackToggle = pack.id;
  head.append(text, group);
  const list = el("div", "agent-pack-tools");
  const rows = pack.tools.map((toolName) => {
    const tool = tools.find((item) => item.name === toolName);
    const row = el("label", "agent-pack-tool");
    row.dataset.agentPackTool = toolName;
    const label = el("span", "agent-choice-text");
    label.append(el("code", "", toolName));
    const meta = el("span", "agent-choice-meta");
    if (tool?.safety) meta.append(el("span", "agent-chip", safetyText[tool.safety] ?? tool.safety));
    if (tool?.runtime_managed) meta.append(el("span", "agent-chip", "управляется runtime"));
    if (!tool) meta.append(el("span", "agent-chip warning", "нет в списке инструментов"));
    if (meta.childElementCount) label.append(meta);
    const input = check(`Включить ${toolName}`, (on) =>
      agentSettings.update((draft) => {
        const next = new Set(draft.tools);
        on ? next.add(toolName) : next.delete(toolName);
        draft.tools = [...next].sort();
      }), view);
    row.append(label, input);
    list.append(row);
    return { input, tool, name: toolName };
  });
  if (!rows.length) list.append(el("p", "settings-hint", "Пакет не сообщил инструментов."));
  block.append(head, list);
  parameters(block, "tool", exported.id, view);
  view.sync((state) => {
    const { active, total, state: mode } = packState(pack, tools, state.draft.tools);
    group.checked = mode === "on";
    group.indeterminate = mode === "mixed";
    group.disabled = state.saving || !total;
    block.dataset.state = mode;
    count.textContent = ` · ${active} из ${total}`;
    for (const { input, tool, name: toolName } of rows) {
      input.checked = !!tool?.runtime_managed || state.draft.tools.includes(toolName);
      input.disabled = state.saving || !!tool?.runtime_managed;
    }
  });
  return block;
}

/** A hook export: opt-in appends it to the ordered hook list. */
function hookRow(exported, view) {
  const row = el("div", "agent-plugin-export");
  row.dataset.agentExport = `hook/${exported.id}`;
  const head = el("div", "agent-hook-head");
  const text = el("span", "agent-choice-text");
  const position = el("span", "agent-chip");
  const meta = el("span", "agent-choice-meta");
  meta.append(el("span", "agent-chip", title("hook")), position);
  text.append(el("strong", "", exported.id), el("span", "settings-hint", exported.description?.trim() || "Описание не задано"), meta);
  const input = check(`Включить обработчик ${exported.id}`, (on) =>
    agentSettings.update((draft) => {
      draft.hooks = draft.hooks.filter((id) => id !== exported.id);
      if (on) draft.hooks.push(exported.id);
    }), view);
  const order = button("Порядок", () => openAgentTarget(slotPage.hook, `[data-agent-hook="${css(exported.id)}"]`), view.signal, "secondary");
  const actions = el("span", "agent-hook-actions");
  actions.append(order, input);
  head.append(text, actions);
  row.append(head);
  parameters(row, "hook", exported.id, view);
  view.sync((state) => {
    const index = state.draft.hooks.indexOf(exported.id);
    input.checked = index >= 0;
    input.disabled = state.saving;
    order.hidden = index < 0;
    position.textContent = index >= 0 ? `№ ${index + 1} в цепочке` : "выключен";
  });
  return row;
}

/** A single-selection export is chosen on its slot page, never here. */
function slotRow(exported, view) {
  const row = el("div", "agent-plugin-export agent-hook-head");
  row.dataset.agentExport = `${exported.slot}/${exported.id}`;
  const text = el("span", "agent-choice-text");
  const meta = el("span", "agent-choice-meta");
  const selected = el("span", "agent-chip", "выбран");
  meta.append(el("span", "agent-chip", title(exported.slot)), selected);
  text.append(el("strong", "", exported.id), el("span", "settings-hint", exported.description?.trim() || "Описание не задано"), meta);
  row.append(text);
  const page = slotPage[exported.slot];
  if (page)
    row.append(button(`Открыть ${title(exported.slot)}`, () =>
      openAgentTarget(page, exported.slot === "model"
        ? `[data-agent-parameters="model/${css(exported.id)}"]`
        : `[data-agent-module="${css(exported.id)}"]`), view.signal, "secondary"));
  view.sync((state) => {
    selected.hidden = exported.slot in state.draft.modules ? state.draft.modules[exported.slot] !== exported.id : !exported.active;
  });
  return row;
}

/** One process component and everything it exports. */
export function pluginCard(plugin, snapshot, view) {
  const card = el("article", "agent-plugin");
  card.dataset.agentPlugin = plugin.id;
  const head = el("div", "agent-choice-text");
  const name = el("h2", "", plugin.id);
  const command = el("code", "agent-plugin-command", plugin.command);
  const counts = el("span", "agent-choice-meta");
  const packs = plugin.exports.filter((item) => item.slot === "tool");
  const hooks = plugin.exports.filter((item) => item.slot === "hook");
  const others = plugin.exports.filter((item) => item.slot !== "tool" && item.slot !== "hook");
  for (const [n, label] of [[packs.length, "пакетов инструментов"], [hooks.length, "обработчиков"], [others.length, "модулей слотов"]])
    if (n) counts.append(el("span", "agent-chip", `${n} ${label}`));
  head.append(name, el("span", "settings-hint", plugin.description?.trim() || "Описание не задано"), command, counts);
  card.append(head);
  for (const [items, heading, render] of [
    [packs, "Пакеты инструментов", (item) => packBlock(plugin, item, snapshot, view)],
    [hooks, "Обработчики", (item) => hookRow(item, view)],
    [others, "Модули слотов", (item) => slotRow(item, view)],
  ]) {
    if (!items.length) continue;
    const section = el("div", "agent-plugin-section");
    section.append(el("h3", "agent-plugin-heading", heading), ...items.map(render));
    card.append(section);
  }
  if (!plugin.exports.length) card.append(el("p", "settings-hint", "Плагин не объявил exports."));
  return card;
}
