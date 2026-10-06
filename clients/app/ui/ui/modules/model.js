import { menu, section, option } from "./menu.js";
// Provider effort ids read as words; an unknown id is shown as is.
const effortNames = {
  auto: "авто",
  none: "выкл",
  minimal: "минимум",
  low: "низкий",
  medium: "средний",
  high: "высокий",
  xhigh: "максимум",
};
const effortName = (effort) => effortNames[effort] ?? effort;
export function mount({ root, services, signal }) {
  const service = services["client.composer"],
    ui = menu(root, "model", "Модель и рассуждение", null, signal);
  let previous = "";
  function render() {
    const state = service.read();
    const key = JSON.stringify([
      state.model,
      state.models,
      state.reasoning,
      state.effort,
      state.efforts,
    ]);
    if (key === previous) return;
    previous = key;
    const name =
      state.models.find((m) => m.name === state.model)?.label ||
      state.model ||
      "Модель из профиля";
    ui.name.textContent = name.length > 32 ? name.slice(0, 31) + "…" : name;
    ui.summary.title = state.model;
    ui.meta.hidden = !state.reasoning;
    ui.meta.textContent = state.reasoning ? effortName(state.effort) : "";
    ui.panel.replaceChildren();
    const models = section(ui.panel, "Модель");
    if (!state.models.length) option(models, state.model || "Из профиля", true);
    for (const model of state.models)
      option(
        models,
        model.label + (model.hidden ? " (скрытая)" : ""),
        state.model === model.name,
        () => service.set("model", model.name),
      );
    if (state.efforts.length) {
      const efforts = section(ui.panel, "Рассуждение");
      efforts.classList.remove("stacked");
      for (const effort of state.efforts) {
        const button = option(efforts, effortName(effort), state.effort === effort, () =>
          service.set("effort", effort),
        );
        button.classList.remove("menu-option-row", "choice-row");
      }
    }
  }
  const stop = service.subscribe(render);
  render();
  return stop;
}
