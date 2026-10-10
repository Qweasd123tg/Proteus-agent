import { agentSettings } from "./store.js";
import { el, mountAgentPage } from "./page.js";
import { slotSection } from "./choices.js";
import { safetyText, slotText } from "./labels.js";
import { openAgentTarget } from "./focus.js";
import { ownerOf } from "./packs.js";

// Descriptions are written for the model; long ones start folded.
const FOLDED_DESCRIPTION = 160;

export function mount(context) {
  mountAgentPage(
    context,
    "Инструменты, которые профиль разрешает агенту, и модуль, отбирающий их для модели.",
    (body, snapshot, view) => {
      const section = el("section", "agent-block");
      const head = el("div", "agent-block-head");
      const title = el("h2", "", "Разрешённые инструменты");
      const count = el("span", "agent-count");
      title.append(count);
      const search = el("input", "agent-search");
      search.type = "search";
      search.placeholder = "Поиск по имени или описанию";
      search.setAttribute("aria-label", "Поиск инструментов");
      head.append(title, search);
      const list = el("div", "agent-tools");
      section.append(head, list);
      body.append(section);
      const known = new Set(snapshot.tools.map((tool) => tool.name));
      const rows = new Map();
      function row(tool) {
        const item = el("label", "agent-tool");
        item.dataset.agentTool = tool.name;
        const input = el("input", "settings-toggle");
        input.type = "checkbox";
        input.setAttribute("aria-label", `Включить ${tool.name}`);
        input.addEventListener(
          "change",
          () =>
            agentSettings.update((draft) => {
              const tools = new Set(draft.tools);
              if (input.checked) tools.add(tool.name);
              else tools.delete(tool.name);
              draft.tools = [...tools].sort();
            }),
          { signal: view.signal },
        );
        const text = el("span", "agent-choice-text");
        const meta = el("span", "agent-choice-meta");
        if (tool.safety) meta.append(el("span", "agent-chip", safetyText[tool.safety] ?? tool.safety));
        if (tool.runtime_managed) meta.append(el("span", "agent-chip", "управляется runtime"));
        // Disabled tools of a plugin are listed but not registered by design.
        const missing = !tool.registered && tool.enabled !== false;
        if (missing) meta.append(el("span", "agent-chip warning", "не зарегистрирован"));
        const owner = ownerOf(tool);
        if (owner) {
          const link = el("button", "agent-owner-link", `${owner.plugin} · ${owner.pack}`);
          link.type = "button";
          link.dataset.agentOwner = `${owner.plugin}/${owner.pack}`;
          link.setAttribute("aria-label", `Плагин ${owner.plugin}, пакет ${owner.pack}`);
          link.addEventListener("click", (event) => {
            event.preventDefault();
            openAgentTarget("agent-plugins", `[data-agent-plugin="${CSS.escape(owner.plugin)}"] [data-agent-pack="${CSS.escape(owner.pack)}"]`);
          }, { signal: view.signal });
          meta.append(link);
        }
        const name = el("code", "", tool.name);
        if (tool.source) name.title = `Источник: ${tool.source}`;
        const description = el("span", "settings-hint agent-tool-description", tool.description || "");
        text.append(name);
        if (tool.description) text.append(description);
        if ((tool.description || "").length > FOLDED_DESCRIPTION) {
          description.classList.add("folded");
          const more = el("button", "agent-tool-more", "Подробнее");
          more.type = "button";
          more.addEventListener("click", (event) => {
            event.preventDefault();
            more.textContent = description.classList.toggle("folded") ? "Подробнее" : "Свернуть";
          }, { signal: view.signal });
          text.append(more);
        }
        text.append(meta);
        item.classList.toggle("unavailable", missing);
        item.append(text, input);
        list.append(item);
        rows.set(tool.name, { item, input, tool });
      }
      for (const tool of snapshot.tools) row(tool);
      const filter = () => {
        const needle = search.value.trim().toLowerCase();
        for (const { item, tool } of rows.values())
          item.hidden = !!needle && !`${tool.name} ${tool.description}`.toLowerCase().includes(needle);
      };
      search.addEventListener("input", filter, { signal: view.signal });
      view.sync((state) => {
        const enabled = new Set(state.draft.tools);
        // A profile may enable a tool that is not registered in this assembly.
        for (const name of enabled)
          if (!known.has(name) && !rows.has(name))
            row({ name, description: "Не найден в текущей сборке", registered: false });
        let active = 0;
        for (const { input, tool } of rows.values()) {
          input.checked = !!tool.runtime_managed || enabled.has(tool.name);
          input.disabled = state.saving || !!tool.runtime_managed;
          if (input.checked) active += 1;
        }
        count.textContent = ` · ${active} из ${rows.size}`;
        filter();
      });
      const [exposureTitle, exposureHint] = slotText.tool_exposure;
      slotSection(body, snapshot, "tool_exposure", view, { title: exposureTitle, hint: exposureHint });
    },
  );
}
