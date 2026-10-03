import { agentSettings } from "./store.js";
import { el, mountAgentPage } from "./page.js";
import { slotSection } from "./choices.js";

export function mount(context) {
  mountAgentPage(
    context,
    "Инструменты, которые профиль разрешает агенту, и модуль, отбирающий их для модели.",
    (body, snapshot, view) => {
      const section = el("section", "agent-block");
      const head = el("div", "agent-block-head");
      const title = el("h2", "", "Инструменты");
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
        for (const value of [tool.safety, tool.source]) if (value) meta.append(el("span", "agent-chip", value));
        if (tool.runtime_managed) meta.append(el("span", "agent-chip", "управляется runtime"));
        if (!tool.registered) meta.append(el("span", "agent-chip warning", "не зарегистрирован"));
        text.append(el("code", "", tool.name), el("span", "settings-hint", tool.description || "Описание не задано"), meta);
        item.classList.toggle("unavailable", !tool.registered);
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
      slotSection(body, snapshot, "tool_exposure", view, { heading: true });
    },
  );
}
