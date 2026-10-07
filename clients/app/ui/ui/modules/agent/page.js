import { agentSettings } from "./store.js";
import { changeLabel } from "./labels.js";

export function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function button(text, onClick, signal, className) {
  const node = el("button", className, text);
  node.type = "button";
  node.addEventListener("click", onClick, { signal });
  return node;
}

function homePath(path) {
  return path?.replace(/^\/home\/[^/]+/, "~") ?? "";
}

function saveBar(service, signal) {
  const element = el("div", "agent-save-bar");
  const info = el("div", "agent-save-info");
  const status = el("span", "agent-save-status");
  status.setAttribute("role", "status");
  status.setAttribute("aria-live", "polite");
  const target = el("code", "agent-save-target");
  info.append(status, target);
  const reload = button("Обновить", () => agentSettings.load(service, true), signal, "secondary");
  const reset = button("Сбросить", () => agentSettings.reset(), signal, "secondary");
  const save = button("Сохранить", () => agentSettings.save(service), signal, "btn-primary");
  save.dataset.agentSave = "";
  const actions = el("div", "agent-save-actions");
  actions.append(reload, reset, save);
  element.append(info, actions);
  function sync(state) {
    const changed = agentSettings.changes();
    const invalid = Object.keys(state.errors).length > 0;
    const writable = !!state.snapshot?.writable;
    let text = "Изменений нет",
      kind = "saved";
    if (state.saving) [text, kind] = ["Сохраняю и проверяю сборку…", "saving"];
    else if (invalid) [text, kind] = ["Исправьте отмеченные значения", "error"];
    else if (state.feedback?.kind === "error") [text, kind] = [state.feedback.text, "error"];
    else if (changed.size)
      [text, kind] = [`Не сохранено: ${[...changed].map(changeLabel).join(", ")}`, "dirty"];
    else if (state.feedback?.kind === "saved") text = state.feedback.text;
    status.textContent = text;
    element.dataset.state = kind;
    target.textContent = state.snapshot
      ? `${homePath(state.snapshot.target_path) || "Файл профиля недоступен"}${writable ? "" : " · только чтение"}`
      : "";
    target.title = state.snapshot?.target_path ?? "";
    const busy = state.saving || state.loading;
    reload.hidden = changed.size > 0;
    reload.disabled = busy;
    reset.disabled = busy || !changed.size;
    save.disabled = busy || !writable || !changed.size || invalid;
  }
  return { element, sync };
}

/**
 * Shared frame of agent pages. `build` creates controls once per loaded
 * profile and registers `sync` callbacks that follow later draft changes.
 */
export function mountAgentPage({ root, services, signal }, intro, build) {
  const service = services["agent.config.builder"];
  const body = el("div", "agent-page-body");
  const bar = saveBar(service, signal);
  root.append(el("p", "settings-section-description", intro), body, bar.element);
  let shown, syncs = [];
  const view = { signal, sync: (fn) => syncs.push(fn) };
  agentSettings.subscribe((state) => {
    const key = state.snapshot ?? (state.loading ? "loading" : state.feedback?.text);
    if (key !== shown) {
      shown = key;
      syncs = [];
      body.replaceChildren();
      if (state.snapshot) build(body, state.snapshot, view);
      else if (state.loading) body.append(el("p", "settings-status", "Загружаю профиль агента…"));
      else if (state.feedback)
        body.append(
          el("p", "settings-status", state.feedback.text),
          button("Повторить", () => agentSettings.load(service, true), signal),
        );
    }
    for (const sync of syncs) sync(state);
    bar.sync(state);
  }, signal);
  agentSettings.load(service);
  const unsubscribe = service.subscribe((error) => agentSettings.refresh(service,error));
  signal.addEventListener("abort",unsubscribe,{once:true});
}
