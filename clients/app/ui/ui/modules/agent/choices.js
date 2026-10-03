import { agentSettings } from "./store.js";
import { el } from "./page.js";
import { parametersEditor } from "./editor.js";
import { slotText } from "./labels.js";

/** A slot section: its implementations and the selected module parameters. */
export function slotSection(container, snapshot, slotId, view, { heading = false } = {}) {
  const slot = snapshot.slots.find((item) => item.id === slotId);
  if (!slot) {
    container.append(el("p", "settings-hint", "Этот слот недоступен в текущей сборке."));
    return;
  }
  if (heading) {
    const [title, description] = slotText[slot.id] ?? [slot.title, slot.responsibility];
    const head = el("div", "agent-group-head");
    head.append(el("h2", "agent-group-title", title), el("p", "settings-hint", description));
    container.append(head);
  }
  const parameters = el("div");
  let editor;
  implementationChoice(container, slot, view, (module) => {
    editor?.dispose();
    editor = module ? parametersEditor(parameters, slot.id, module, view.signal) : undefined;
  });
  container.append(parameters);
  view.sync((state) => editor?.sync(state));
}

/** Radio cards for implementations of one host-defined slot. */
export function implementationChoice(container, slot, view, onSelected) {
  const section = el("section", "agent-block");
  section.append(el("h2", "", "Реализация"));
  if (!slot.modules.length) {
    section.append(
      el(
        "p",
        "settings-hint",
        "В профиле нет модулей для этого слота. Подключите модуль в разделе components файла профиля.",
      ),
    );
    container.append(section);
    return;
  }
  const list = el("div", "agent-choices");
  list.setAttribute("role", "radiogroup");
  const inputs = new Map();
  for (const module of slot.modules) {
    const card = el("label", "agent-choice");
    card.dataset.agentModule = module.id;
    const input = el("input");
    input.type = "radio";
    input.name = `agent-slot-${slot.id}`;
    input.value = module.id;
    input.addEventListener(
      "change",
      () => agentSettings.update((draft) => (draft.modules[slot.id] = module.id)),
      { signal: view.signal },
    );
    const text = el("span", "agent-choice-text");
    text.append(
      el("strong", "", module.id),
      el("span", "settings-hint", module.description?.trim() || "Описание не задано"),
    );
    // Source and version tell where the module comes from; transport
    // capabilities stay in the diagnostics architecture view.
    const meta = el("span", "agent-choice-meta");
    for (const value of new Set([module.source, module.version].filter(Boolean)))
      meta.append(el("span", "agent-chip", value));
    if (meta.childElementCount) text.append(meta);
    card.append(input, text);
    list.append(card);
    inputs.set(module.id, input);
  }
  section.append(list);
  container.append(section);
  let selected;
  view.sync((state) => {
    const current = state.draft.modules[slot.id];
    for (const [id, input] of inputs) {
      input.checked = id === current;
      input.closest(".agent-choice").classList.toggle("active", id === current);
      input.disabled = state.saving;
    }
    if (current !== selected) {
      selected = current;
      onSelected?.(current);
    }
  });
}
