import { menu, section, option } from "./menu.js";
export function mount({ root, services, signal }) {
  const service = services["client.composer"],
    ui = menu(root, "access", "Режим доступа", "shield", signal);
  ui.meta.hidden = true;
  let previous;
  function render() {
    const state = service.read();
    if (previous === state.mode) return;
    previous = state.mode;
    const selected = state.modes.find((m) => m.value === state.mode);
    ui.name.textContent = selected?.label || state.mode;
    ui.summary.title = selected?.description || "";
    ui.panel.replaceChildren();
    const list = section(ui.panel, "Режим доступа");
    for (const mode of state.modes)
      option(
        list,
        mode.label,
        mode.value === state.mode,
        () => service.set("mode", mode.value),
        mode.description,
      );
  }
  const stop = service.subscribe(render);
  render();
  return stop;
}
