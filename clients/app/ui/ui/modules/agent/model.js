import { agentSettings } from "./store.js";
import { el, mountAgentPage } from "./page.js";
import { parametersEditor } from "./editor.js";

export function mount(context) {
  mountAgentPage(
    context,
    "Модель профиля по умолчанию. Модель и уровень рассуждения текущего чата можно сменить в поле ввода.",
    (body, snapshot, view) => {
      const section = el("section", "agent-block");
      section.append(el("h2", "", "Модель"));
      const list = el("div", "agent-choices");
      list.setAttribute("role", "radiogroup");
      const inputs = new Map();
      for (const provider of snapshot.providers) {
        const card = el("label", "agent-choice");
        card.dataset.agentProvider = provider.id;
        const input = el("input");
        input.type = "radio";
        input.name = "agent-provider";
        input.value = provider.id;
        input.addEventListener(
          "change",
          () => agentSettings.update((draft) => (draft.provider = provider.id)),
          { signal: view.signal },
        );
        const text = el("span", "agent-choice-text");
        const meta = el("span", "agent-choice-meta");
        meta.append(el("span", "agent-chip", provider.id), el("span", "agent-chip", provider.provider));
        text.append(el("strong", "", provider.label || provider.model), el("span", "settings-hint", provider.model), meta);
        card.append(input, text);
        list.append(card);
        inputs.set(provider.id, input);
      }
      if (!snapshot.providers.length)
        section.append(el("p", "settings-hint", "В профиле нет настроенных провайдеров моделей."));
      section.append(list);
      body.append(section);
      // Parameters belong to the model module export shared by its providers.
      const parameters = el("div");
      body.append(parameters);
      let editor, module;
      view.sync((state) => {
        for (const [id, input] of inputs) {
          input.checked = id === state.draft.provider;
          input.closest(".agent-choice").classList.toggle("active", input.checked);
          input.disabled = state.saving;
        }
        const next = snapshot.providers.find((p) => p.id === state.draft.provider)?.provider;
        if (next !== module) {
          module = next;
          editor?.dispose();
          editor = next ? parametersEditor(parameters, "model", next, view.signal) : undefined;
        }
        editor?.sync(state);
      });
    },
  );
}
