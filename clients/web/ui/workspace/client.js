import { mountControlPlacement } from "../modules/placement.js";
import { createWorkspace } from "../../extensions/workspace.js";
let board, attachment;
export function clientWorkspace() {
  if (board) return board;
  const target = document.querySelector("[data-client-workspace]");
  if (!target) throw Error("Область вкладок ещё не создана");
  board = createWorkspace(target, { storage: localStorage });
  const records = [...target.querySelectorAll("[data-client-view]")].map(
    (element) => ({
      id: `client:${element.dataset.clientView}`,
      client: true,
      owned: true,
      element,
      collapsed: !board.has(`client:${element.dataset.clientView}`),
      manifest: { name: element.dataset.workspaceTitle },
      onVisibility(shown) {
        document.dispatchEvent(
          new CustomEvent("proteus-workspace-visibility", {
            detail: `${element.dataset.clientView}:${shown ? "shown" : "hidden"}`,
          }),
        );
      },
      onSelect() {
        document.dispatchEvent(
          new CustomEvent("proteus-workspace-route", {
            detail: element.dataset.clientView,
          }),
        );
      },
    }),
  );
  attachment = board.connect("client", {
    select(id) {
      const r = records.find((r) => r.id === id);
      if (r) {
        r.collapsed = false;
        attachment.update(records);
      }
    },
    close(id) {
      const r = records.find((r) => r.id === id);
      if (r) {
        r.collapsed = true;
        attachment.update(records);
      }
    },
  });
  attachment.update(records);
  function reveal(view) {
    const r = records.find((r) => r.id === `client:${view}`);
    if (!r) return;
    r.collapsed = false;
    attachment.update(records);
    board.reveal(r.id);
  }
  const controller = new AbortController(),
    { signal } = controller;
  document.addEventListener("proteus-workspace-open", (e) => reveal(e.detail), {
    signal,
  });
  const titles = new MutationObserver(() => {
    for (const r of records) r.manifest.name = r.element.dataset.workspaceTitle;
    attachment.update(records);
  });
  for (const r of records)
    titles.observe(r.element, {
      attributes: true,
      attributeFilter: ["data-workspace-title"],
    });
  const stopControls = mountControlPlacement(target);
  const stop = board.stop;
  board.stop = () => {
    stopControls();
    controller.abort();
    titles.disconnect();
    for (const r of records) {
      r.element.hidden = false;
      r.element.inert = false;
      target.append(r.element);
    }
    stop();
    board = attachment = undefined;
  };
  const route = new URL(location.href),
    requestedView = route.searchParams.get("workspace_view");
  if (["chat", "settings"].includes(requestedView)) {
    route.searchParams.delete("workspace_view");
    history.replaceState(history.state, "", route);
    reveal(requestedView);
  } else if (location.pathname === "/settings") reveal("settings");
  return board;
}
export function mountClientWorkspace() {
  const current = clientWorkspace();
  return () => current.stop();
}
export function revealClientView(view) {
  document.dispatchEvent(
    new CustomEvent("proteus-workspace-open", { detail: view }),
  );
}
