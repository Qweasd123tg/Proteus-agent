import { agentSettings } from "./store.js";
import { el } from "./page.js";
import { parametersEditor } from "./editor.js";

/**
 * A slot section: its implementations and the selected module parameters.
 * Pages that host several slots name the choice after the slot instead of
 * a generic «Реализация».
 */
export function slotSection(container, snapshot, slotId, view, { title, hint } = {}) {
  const slot = snapshot.slots.find((item) => item.id === slotId);
  if (!slot) {
    container.append(el("p", "settings-hint", "Этот слот недоступен в текущей сборке."));
    return;
  }
  const parameters = el("div");
  let editor;
  implementationChoice(container, slot, view, (module) => {
    editor?.dispose();
    editor = module ? parametersEditor(parameters, slot.id, module, view.signal) : undefined;
  }, { title, hint });
  container.append(parameters);
  view.sync((state) => editor?.sync(state));
}

/** Radio rows for implementations of one host-defined slot. */
export function implementationChoice(container, slot, view, onSelected, { title = "Реализация", hint } = {}) {
  const section = el("section", "agent-block");
  const head = el("div", "agent-block-title");
  head.append(el("h2", "", title));
  if (hint) head.append(el("p", "settings-hint", hint));
  section.append(head);
  if (!slot.modules.length) {
    section.append(
      el(
        "p",
        "agent-empty",
        "Профиль не подключает модулей для этого слота. Реализация появится здесь, когда компонент профиля предоставит такой export.",
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
    text.append(el("strong", "", module.id));
    const description = module.description?.trim();
    if (description) text.append(el("span", "settings-hint", description));
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
