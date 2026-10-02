import { hasSurface } from "../../extensions/contract.js";
import { icon } from "../../extensions/icons.js";
import { mountModule } from "./host.js";
import { watchViewMotion } from "../view-motion.js";
export function mountSettings(root, registry, services, requested) {
  const controller = new AbortController(),
    signal = controller.signal,
    mounted = new Map(),
    buttons = new Map();
  root.className = "settings-page";
  const nav = document.createElement("nav");
  nav.className = "settings-nav";
  nav.setAttribute("aria-label", "Разделы настроек");
  const back = document.createElement("a");
  back.className = "settings-back";
  back.href = "/";
  back.append(
    icon("arrow-left"),
    document.createTextNode("Вернуться к работе"),
  );
  back.addEventListener(
    "click",
    (e) => {
      if (e.button === 0 && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        document.dispatchEvent(
          new CustomEvent("proteus-client-navigation", { detail: "workspace" }),
        );
      }
    },
    { signal },
  );
  nav.append(back);
  const content = document.createElement("div");
  content.className = "settings-content";
  const header = document.createElement("header");
  header.className = "settings-toolbar";
  const title = document.createElement("h1");
  header.append(title);
  content.append(header);
  root.append(nav, content);
  let selected =
    requested ||
    new URL(location.href).searchParams.get("settings_module") ||
    "appearance";
  function select(id, history = true) {
    selected = id;
    if (history) {
      const url = new URL(location.href);
      url.searchParams.set("settings_module", id);
      window.history.replaceState(null, "", url);
    }
    render();
  }
  function render() {
    const state = registry.state();
    const pages = state.records.filter(
      (r) => r.enabled && r.manifest && hasSurface(r.manifest, "settings"),
    );
    if (!pages.some((r) => r.id === selected)) selected = "extensions";
    for (const [id, item] of mounted)
      if (!pages.some((r) => r === item.record)) {
        item.stop();
        item.section.remove();
        mounted.delete(id);
      }
    for (const [id, button] of buttons)
      if (!pages.some((r) => r.id === id)) {
        button.remove();
        buttons.delete(id);
      }
    nav.querySelectorAll(".settings-nav-label").forEach((el) => el.remove());
    for (const [group, label] of [
      ["settings", "Настройки"],
      ["diagnostics", "Диагностика"],
    ]) {
      const items = pages.filter(
        (r) => (r.manifest.navigation?.group || "settings") === group,
      );
      if (!items.length) continue;
      const heading = document.createElement("span");
      heading.className = "settings-nav-label";
      heading.textContent = label;
      nav.append(heading);
      for (const record of items) {
        let button = buttons.get(record.id);
        if (!button) {
          button = document.createElement("button");
          button.type = "button";
          button.dataset.settingsSection = record.id;
          button.append(
            icon(record.manifest.navigation?.icon || "modules"),
            document.createTextNode(record.manifest.name),
          );
          button.addEventListener("click", () => select(record.id), { signal });
          buttons.set(record.id, button);
        }
        button.classList.toggle("active", record.id === selected);
        button.setAttribute("aria-pressed", String(record.id === selected));
        nav.append(button);
      }
    }
    const record = pages.find((r) => r.id === selected);
    if (!record) return;
    title.textContent = record.manifest.name;
    root.dataset.settingsModule = selected;
    content.classList.toggle(
      "diagnostic-settings",
      record.manifest.navigation?.group === "diagnostics",
    );
    if (!mounted.has(selected)) {
      const section = document.createElement("section");
      section.className = "settings-section";
      section.dataset.modulePage = record.id;
      section.hidden = true;
      content.append(section);
      const stopMotion = watchViewMotion(section, { signal });
      // Register before mount: a module may subscribe to the same registry.
      const item = { record, section, stop: () => {} };
      mounted.set(selected, item);
      const stopModule = mountModule(
        section,
        record,
        registry,
        services,
        "settings",
      );
      item.stop = () => {
        stopMotion();
        stopModule();
      };
    }
    for (const [id, item] of mounted) {
      if (id !== selected && !item.section.hidden)
        item.section.dispatchEvent(new Event("module-hide", { bubbles: true }));
      item.section.hidden = id !== selected;
      item.section.inert = id !== selected;
    }
  }
  window.addEventListener(
    "popstate",
    () =>
      select(
        new URL(location.href).searchParams.get("settings_module") ||
          "appearance",
        false,
      ),
    { signal },
  );
  document.addEventListener(
    "proteus-select-settings-module",
    (e) => select(e.detail),
    { signal },
  );
  root.closest("[data-client-view]")?.addEventListener(
    "module-hide",
    () => {
      for (const item of mounted.values())
        if (!item.section.hidden)
          item.section.dispatchEvent(new Event("module-hide"));
    },
    { signal },
  );
  const unsubscribe = registry.subscribe(render);
  void registry.start();
  return () => {
    controller.abort();
    unsubscribe();
    for (const item of mounted.values()) item.stop();
    root.replaceChildren();
  };
}
