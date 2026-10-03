import { hasSurface } from "../../extensions/contract.js";
export function selectionButtons(root, record, registry, signal) {
  for (const slot of ["composer-model", "composer-access"])
    if (hasSurface(record.manifest, slot)) {
      const button = document.createElement("button");
      button.type = "button";
      button.dataset.selectSlot = slot;
      button.dataset.moduleId = record.id;
      const selected = registry.state().slots?.[slot] === record.id;
      button.textContent = selected ? "Выбран" : "Использовать";
      button.disabled = !record.enabled || selected;
      button.setAttribute("aria-pressed", String(selected));
      button.addEventListener("click", () => registry.select(slot, record.id), {
        signal,
      });
      root.append(button);
    }
}
export function mountBuiltinManagement(root, registry) {
  if (!registry.resetCore) return () => {};
  const controller = new AbortController(),
    rows = new Map();
  let choicesController;
  const heading = document.createElement("p");
  heading.className = "settings-section-description";
  heading.textContent =
    "Основные части клиента: настройки, диагностика и управление чатом. Отключайте их только при необходимости.";
  root.append(heading);
  const list = document.createElement("div");
  list.className = "builtin-module-list";
  root.append(list);
  const reset = document.createElement("button");
  reset.type = "button";
  reset.textContent = "Восстановить встроенные расширения";
  reset.addEventListener("click", () => registry.resetCore(), {
    signal: controller.signal,
  });
  root.append(reset);
  const unsubscribe = registry.subscribe(() => {
    choicesController?.abort();
    choicesController = new AbortController();
    // Agent settings are part of the host, not optional client features.
    for (const record of registry
      .state()
      .records.filter(
        (r) => r.builtin && r.manifest.navigation?.group !== "agent",
      )) {
      let row = rows.get(record.id);
      if (!row) {
        row = document.createElement("div");
        row.className = "settings-row";
        row.dataset.builtinModule = record.id;
        const label = document.createElement("label");
        label.className = "settings-label";
        const name = document.createElement("strong");
        name.textContent = record.manifest.name;
        const hint = document.createElement("span");
        hint.className = "settings-hint";
        hint.textContent = record.required
          ? "Обязательное расширение — управление всегда доступно."
          : record.manifest.description;
        label.append(name, hint);
        const input = document.createElement("input");
        input.type = "checkbox";
        input.className = "settings-toggle";
        input.id = "module-" + record.id;
        input.setAttribute("aria-label", "Включить: " + record.manifest.name);
        label.htmlFor = input.id;
        input.disabled = record.required;
        input.addEventListener(
          "change",
          () => registry.update(record.id, { enabled: input.checked }),
          { signal: controller.signal },
        );
        const choices = document.createElement("span");
        choices.className = "module-selection";
        row.append(label, choices, input);
        list.append(row);
        rows.set(record.id, row);
      }
      row.querySelector("input").checked = record.enabled;
      const choices = row.querySelector(".module-selection");
      choices.replaceChildren();
      selectionButtons(choices, record, registry, choicesController.signal);
    }
  });
  return () => {
    controller.abort();
    choicesController?.abort();
    unsubscribe();
    root.replaceChildren();
  };
}
