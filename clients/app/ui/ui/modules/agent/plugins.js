import { el, mountAgentPage } from "./page.js";
import { pluginCard } from "./plugin-card.js";

// Agent Plugins 1.0 packages bring skills and MCP servers; they are not
// process components and are only listed here with their own label.
function addonPackages(body, view) {
  const section = el("section", "agent-block");
  section.dataset.agentAddonPlugins = "";
  section.append(
    el("h2", "", "Пакеты Agent Plugins"),
    el("p", "settings-hint", "Пакеты навыков и MCP-серверов. Это не процессные плагины: они не дают tools exports и обработчиков."),
  );
  const list = el("div", "agent-hooks");
  section.append(list);
  body.append(section);
  let shown;
  view.sync((state) => {
    const packages = state.draft.addon_settings?.addons?.plugins ?? [];
    const key = JSON.stringify(packages);
    if (key === shown) return;
    shown = key;
    section.hidden = !packages.length;
    list.replaceChildren(...packages.map((item) => {
      const row = el("div", "agent-hook agent-hook-head");
      const text = el("span", "agent-choice-text");
      const meta = el("span", "agent-choice-meta");
      meta.append(el("span", "agent-chip", "skills и MCP"), el("span", "agent-chip", item.enabled ? "включён" : "выключен"));
      text.append(el("code", "", item.path), meta);
      row.append(text);
      return row;
    }));
  });
}

export function mount(context) {
  mountAgentPage(
    context,
    "Плагины — процессные компоненты профиля. Один плагин может давать пакеты инструментов, обработчики и модули слотов.",
    (body, snapshot, view) => {
      const list = el("div", "agent-plugins");
      list.append(...snapshot.plugins.map((plugin) => pluginCard(plugin, snapshot, view)));
      if (!snapshot.plugins.length) list.append(el("p", "settings-hint", "В профиле нет процессных плагинов."));
      body.append(list);
      addonPackages(body, view);
    },
  );
}
