import { agentSettings } from "./store.js";
import { button, el, mountAgentPage } from "./page.js";
import { diffDrafts, draftFromSnapshot } from "./draft.js";
import { describeChanges, revisionDraft, revisionTime } from "./revisions.js";

export function mount(context) {
  const service = context.services["agent.config.builder"];
  mountAgentPage(
    context,
    "Каждое сохранение настроек агента запоминает состояние профиля, которое оно заменило. «Вернуть как было» загружает это состояние в черновик: проверьте изменения внизу и сохраните. Откат тоже попадает в историю.",
    (body, snapshot, view) => {
      const status = el("p", "settings-status", "Загружаю историю…");
      status.setAttribute("role", "status");
      const list = el("div", "agent-revisions");
      body.append(status, list);
      const saved = draftFromSnapshot(snapshot);
      const rows = [];
      const refresh = (state) => {
        for (const row of rows) {
          const current = diffDrafts(row.draft, saved).size === 0;
          const loaded = !current && diffDrafts(row.draft, state.draft).size === 0;
          row.restore.textContent = current ? "Текущее состояние" : loaded ? "В черновике" : "Вернуть как было";
          row.restore.disabled = state.saving || current || loaded;
        }
      };
      service.history().then(
        ({ revisions }) => {
          status.textContent = revisions.length
            ? ""
            : "Сохранений пока не было: история появится после первого изменения настроек агента.";
          status.className = revisions.length ? "settings-status" : "agent-empty";
          status.hidden = revisions.length > 0;
          // Each save changed its replaced state into the next newer one.
          let newer = saved;
          for (const revision of revisions) {
            const before = revisionDraft(snapshot, revision.state);
            const item = el("article", "agent-revision");
            item.dataset.agentRevision = revision.id;
            const head = el("div", "agent-hook-head");
            const title = el("span", "agent-choice-text");
            title.append(el("strong", "", `Сохранение ${revisionTime(revision.replaced_at_ms)}`));
            const restore = button(
              "Вернуть как было",
              () => agentSettings.restore(revision.state),
              view.signal,
              "secondary",
            );
            restore.setAttribute("aria-label", `Вернуть состояние до сохранения ${revisionTime(revision.replaced_at_ms)}`);
            head.append(title, restore);
            const changes = el("ul", "agent-revision-changes");
            for (const change of describeChanges(before, newer)) {
              const line = el("li");
              line.dataset.change = change.key;
              line.append(el("strong", "", change.label));
              if (change.detail) line.append(document.createTextNode(`: ${change.detail}`));
              changes.append(line);
            }
            if (!changes.childElementCount)
              changes.append(el("li", "settings-hint", "Изменился только порядок значений"));
            item.append(head, changes);
            list.append(item);
            rows.push({ draft: before, restore });
            newer = before;
          }
          refresh(agentSettings.state());
        },
        (error) => {
          if (!view.signal.aborted) status.textContent = `Не удалось загрузить историю: ${error.message}`;
        },
      );
      view.sync(refresh);
    },
  );
}
