import { enableHorizontalReorder } from "../../extensions/horizontal-reorder.js";
import { popup } from "../popup.js";
const key = "proteus.client.controls.layout";
const zones = ["composer-start", "composer-end", "header"];
const labels = [
  "Слева под полем ввода",
  "Справа под полем ввода",
  "В верхней панели",
];
// Placement belongs to a surface, so replacing a module preserves its location.
export function mountControlPlacement(target) {
  const controller = new AbortController(),
    { signal } = controller;
  const docks = [...document.querySelectorAll("[data-module-zone]")],
    items = [];
  const menu = popup("module-placement-menu", "Переместить расширение", {
    manual: true,
  });
  const status = document.createElement("span");
  status.className = "workspace-status";
  status.setAttribute("role", "status");
  target.append(status);
  let layout = [];
  try {
    const raw = localStorage.getItem(key);
    if (raw) layout = JSON.parse(raw);
    if (
      !Array.isArray(layout) ||
      layout.some(
        (r, i) =>
          !r ||
          !zones.includes(r.zone) ||
          typeof r.id !== "string" ||
          layout.findIndex((x) => x.id === r.id) !== i,
      )
    )
      throw Error("Некорректное расположение расширений");
  } catch (error) {
    status.textContent = error.message;
    layout = [];
  }
  const remember = () => {
    try {
      localStorage.setItem(
        key,
        JSON.stringify(
          docks.flatMap((d) =>
            [...d.querySelectorAll(".client-module-movable")].map((row) => ({
              id: row.dataset.controlId,
              zone: d.dataset.moduleZone,
            })),
          ),
        ),
      );
    } catch {
      status.textContent = "Не удалось сохранить расположение расширений";
    }
  };
  function move(row, dock, before = null) {
    for (const details of row.querySelectorAll("details[open]"))
      details.open = false;
    dock.insertBefore(row, before);
    remember();
    row.querySelector(".module-drag-handle").focus({ preventScroll: true });
  }
  function choices(row, anchor, event) {
    event?.preventDefault();
    menu.show(
      zones.map((zone, i) =>
        menu.action(labels[i], "arrow-right", () =>
          move(
            row,
            docks.find((d) => d.dataset.moduleZone === zone),
          ),
        ),
      ),
      anchor,
      event ? { x: event.clientX, y: event.clientY } : undefined,
    );
  }
  for (const root of document.querySelectorAll("[data-client-slot]")) {
    const id = root.dataset.clientSlot,
      origin = document.createComment("client module home");
    root.before(origin);
    const row = document.createElement("div");
    row.className = "client-module-movable";
    row.dataset.controlId = id;
    const handle = document.createElement("button");
    handle.type = "button";
    handle.className = "module-drag-handle";
    handle.textContent = "⠿";
    const label =
      id === "composer-model"
        ? "Модель и рассуждение"
        : id === "composer-access"
          ? "Режим доступа"
          : id;
    handle.title = `Переместить: ${label}`;
    handle.setAttribute("aria-label", handle.title);
    handle.addEventListener("click", () => choices(row, handle), { signal });
    row.addEventListener("contextmenu", (e) => choices(row, handle, e), {
      signal,
    });
    root.before(row);
    row.append(handle, root);
    items.push({ root, row, origin });
  }
  for (const entry of layout) {
    const row = items.find((i) => i.row.dataset.controlId === entry.id)?.row;
    if (row) docks.find((d) => d.dataset.moduleZone === entry.zone).append(row);
  }
  function dragging(value) {
    for (const dock of docks)
      dock.classList.toggle("module-drop-target", value);
  }
  for (const dock of docks)
    enableHorizontalReorder(dock, {
      itemSelector: ".client-module-movable",
      handleSelector: ".module-drag-handle",
      id: (row) => row.dataset.controlId,
      lists: () => docks,
      signal,
      onStart() {
        menu.hide();
        dragging(true);
        for (const d of document.querySelectorAll(".composer-menu[open]"))
          d.open = false;
      },
      onFinish() {
        dragging(false);
      },
      commit(id, before, target) {
        const row = items.find(
          (item) => item.row.dataset.controlId === id,
        )?.row;
        const next = items.find(
          (item) => item.row.dataset.controlId === before,
        )?.row;
        if (row)
          target.insertBefore(row, next?.parentNode === target ? next : null);
        remember();
      },
    });
  return () => {
    controller.abort();
    menu.dispose();
    status.remove();
    for (const { root, row, origin } of items) {
      origin.replaceWith(root);
      row.remove();
    }
  };
}
