import { agentSettings } from "./store.js";
import { el, mountAgentPage } from "./page.js";
import { slotSection } from "./choices.js";
import { permissionText } from "./labels.js";

export function mount(context) {
  mountAgentPage(
    context,
    "Режим прав профиля и политика, которая решает, какие действия требуют подтверждения. Режим текущего чата меняется в поле ввода.",
    (body, snapshot, view) => {
      const section = el("section", "agent-block");
      section.append(el("h2", "", "Режим прав по умолчанию"));
      const list = el("div", "agent-choices");
      list.setAttribute("role", "radiogroup");
      const inputs = new Map();
      for (const mode of snapshot.permission_modes) {
        const [title, description] = permissionText[mode] ?? [mode, ""];
        const card = el("label", "agent-choice");
        card.dataset.agentMode = mode;
        const input = el("input");
        input.type = "radio";
        input.name = "agent-permission-mode";
        input.value = mode;
        input.addEventListener(
          "change",
          () => agentSettings.update((draft) => (draft.mode = mode)),
          { signal: view.signal },
        );
        const text = el("span", "agent-choice-text");
        text.append(el("strong", "", title), el("span", "settings-hint", description));
        card.append(input, text);
        list.append(card);
        inputs.set(mode, input);
      }
      section.append(list);
      body.append(section);
      view.sync((state) => {
        for (const [mode, input] of inputs) {
          input.checked = mode === state.draft.mode;
          input.closest(".agent-choice").classList.toggle("active", input.checked);
          input.disabled = state.saving;
        }
      });
      slotSection(body, snapshot, "policy", view, { heading: true });
    },
  );
}
