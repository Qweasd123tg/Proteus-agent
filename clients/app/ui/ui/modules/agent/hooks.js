import { agentSettings } from "./store.js";
import { button, el, mountAgentPage } from "./page.js";
import { parametersEditor } from "./editor.js";

const move = (list, index, step) => {
  const next = [...list];
  [next[index], next[index + step]] = [next[index + step], next[index]];
  return next;
};

export function mount(context) {
  mountAgentPage(
    context,
    "Обработчики дополняют шаги агента: проверяют вызовы инструментов, добавляют инструкции и проверяют завершение. Вызываются по порядку списка.",
    (body, snapshot, view) => {
      const describe = (id) => snapshot.hook_modules.find((module) => module.id === id);
      const enabledSection = el("section", "agent-block");
      enabledSection.append(el("h2", "", "Включены"));
      const enabledList = el("div", "agent-hooks");
      enabledSection.append(enabledList);
      const availableSection = el("section", "agent-block");
      availableSection.append(el("h2", "", "Доступны"));
      const availableList = el("div", "agent-hooks");
      availableSection.append(availableList);
      body.append(enabledSection, availableSection);
      let shown, editors = [];
      view.sync((state) => {
        const hooks = state.draft.hooks;
        const key = JSON.stringify(hooks) + state.saving;
        if (key !== shown) {
          shown = key;
          for (const editor of editors) editor.dispose();
          editors = [];
          enabledList.replaceChildren();
          availableList.replaceChildren();
          hooks.forEach((id, index) => {
            const row = el("div", "agent-hook");
            row.dataset.agentHook = id;
            const head = el("div", "agent-hook-head");
            const text = el("span", "agent-choice-text");
            text.append(
              el("strong", "", `${index + 1}. ${id}`),
              el("span", "settings-hint", describe(id)?.description?.trim() || (describe(id) ? "Описание не задано" : "Модуль не найден в текущей сборке")),
            );
            const actions = el("span", "agent-hook-actions");
            const update = (next) => agentSettings.update((draft) => (draft.hooks = next));
            const up = button("↑", () => update(move(hooks, index, -1)), view.signal, "secondary");
            up.disabled = state.saving || index === 0;
            up.setAttribute("aria-label", `Поднять ${id}`);
            const down = button("↓", () => update(move(hooks, index, 1)), view.signal, "secondary");
            down.disabled = state.saving || index === hooks.length - 1;
            down.setAttribute("aria-label", `Опустить ${id}`);
            const off = button("Отключить", () => update(hooks.filter((item) => item !== id)), view.signal, "secondary");
            off.disabled = state.saving;
            actions.append(up, down, off);
            head.append(text, actions);
            const details = el("details", "agent-hook-parameters");
            details.append(el("summary", "", "Параметры"));
            row.append(head, details);
            enabledList.append(row);
            editors.push(parametersEditor(details, "hook", id, view.signal));
          });
          if (!hooks.length) enabledList.append(el("p", "settings-hint", "Обработчики не включены."));
          const available = snapshot.hook_modules.filter((module) => !hooks.includes(module.id));
          for (const module of available) {
            const row = el("div", "agent-hook agent-hook-head");
            row.dataset.agentHookAvailable = module.id;
            const text = el("span", "agent-choice-text");
            text.append(el("strong", "", module.id), el("span", "settings-hint", module.description?.trim() || "Описание не задано"));
            const add = button("Включить", () => agentSettings.update((draft) => draft.hooks.push(module.id)), view.signal, "secondary");
            add.disabled = state.saving;
            row.append(text, add);
            availableList.append(row);
          }
          if (!available.length)
            availableList.append(el("p", "settings-hint", snapshot.hook_modules.length ? "Все обработчики включены." : "В профиле нет модулей обработчиков."));
        }
        for (const editor of editors) editor.sync(state);
      });
    },
  );
}
