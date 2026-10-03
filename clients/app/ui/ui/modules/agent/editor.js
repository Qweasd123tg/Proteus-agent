import { agentSettings } from "./store.js";
import { moduleText, parseParameters } from "./draft.js";
import { button, el } from "./page.js";

const pretty = (value) => JSON.stringify(value, null, 2);

function parseNumber(text) {
  const value = Number(text.trim());
  return text.trim() && Number.isFinite(value) ? value : undefined;
}

function parseLenient(text) {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/**
 * Opaque module parameters shown as typed rows with a raw JSON fallback.
 * Rows follow the current values until modules describe their own schema.
 */
export function parametersEditor(host, slot, module, signal) {
  const owned = new Set();
  const errorKey = (field) => `${slot}\u001f${module}\u001f${field}`;
  let raw = false,
    written,
    saving = false;
  const section = el("section", "agent-block agent-parameters");
  section.dataset.agentParameters = `${slot}/${module}`;
  const head = el("div", "agent-block-head");
  const title = el("h2", "", "Параметры");
  const count = el("span", "agent-count");
  title.append(count);
  const toggle = button("JSON", () => {
    raw = !raw;
    rebuild(current());
  }, signal, "secondary agent-mode-toggle");
  head.append(title, toggle);
  const body = el("div", "agent-parameters-body");
  section.append(head, el("p", "settings-hint", `Модуль ${module}. Значения сохраняются в профиле как JSON.`), body);
  host.append(section);

  const current = () => moduleText(agentSettings.state().draft, slot, module);
  function setError(field, message) {
    const key = errorKey(field);
    if (message) owned.add(key);
    else owned.delete(key);
    agentSettings.setError(key, message);
  }
  function clearFieldErrors() {
    for (const key of [...owned]) if (!key.endsWith("\u001f@raw")) setError(key.split("\u001f")[2]);
  }
  function write(text) {
    written = text;
    agentSettings.update((draft) => {
      draft.texts[slot] ??= {};
      draft.texts[slot][module] = text;
    });
  }
  function change(mutate) {
    const object = parseParameters(current()).value ?? {};
    mutate(object);
    write(pretty(object));
    count.textContent = ` · ${Object.keys(object).length}`;
  }

  function control(name, value) {
    if (typeof value === "boolean") {
      const input = el("input", "settings-toggle");
      input.type = "checkbox";
      input.checked = value;
      input.addEventListener("change", () => change((o) => (o[name] = input.checked)), { signal });
      return input;
    }
    const input = el("input", typeof value === "string" ? "" : "agent-json-value");
    input.type = "text";
    input.spellcheck = false;
    input.value = typeof value === "string" ? value : JSON.stringify(value);
    if (typeof value === "number") input.inputMode = "decimal";
    input.addEventListener("input", () => {
      if (typeof value === "string") return change((o) => (o[name] = input.value));
      let next;
      if (typeof value === "number") next = parseNumber(input.value);
      else
        try {
          next = JSON.parse(input.value);
        } catch {
          next = undefined;
        }
      if (next === undefined) {
        setError(name, typeof value === "number" ? "Введите число" : "Введите корректное JSON-значение");
        input.closest(".agent-parameter").dataset.invalid = "";
        return;
      }
      setError(name);
      delete input.closest(".agent-parameter").dataset.invalid;
      change((o) => (o[name] = next));
    }, { signal });
    return input;
  }

  function addRow() {
    const row = el("div", "agent-parameter agent-parameter-add");
    const key = el("input");
    key.placeholder = "Новый параметр";
    key.setAttribute("aria-label", "Имя параметра");
    key.addEventListener("input", () => key.setCustomValidity(""), { signal });
    const value = el("input", "agent-json-value");
    value.placeholder = "Значение: строка или JSON";
    value.setAttribute("aria-label", "Значение параметра");
    const add = button("Добавить", () => {
      const name = key.value.trim();
      if (!name) return key.focus();
      const object = parseParameters(current()).value ?? {};
      if (Object.hasOwn(object, name)) {
        key.setCustomValidity("Такой параметр уже есть");
        key.reportValidity();
        return;
      }
      key.setCustomValidity("");
      change((o) => (o[name] = parseLenient(value.value.trim())));
      rebuild(current());
    }, signal, "secondary");
    row.append(key, value, add);
    return row;
  }

  function rebuild(text) {
    written = text;
    clearFieldErrors();
    body.replaceChildren();
    const parsed = parseParameters(text);
    if (raw || parsed.error) {
      raw = true;
      const area = el("textarea", "agent-raw");
      area.spellcheck = false;
      area.value = text;
      area.rows = Math.min(18, Math.max(4, text.split("\n").length + 1));
      area.setAttribute("aria-label", `Параметры ${module} в JSON`);
      const error = el("p", "agent-error");
      const validate = () => {
        const result = parseParameters(area.value);
        error.textContent = result.error ?? "";
        setError("@raw", result.error);
        toggle.disabled = saving || !!result.error;
      };
      area.addEventListener("input", () => {
        write(area.value);
        validate();
      }, { signal });
      body.append(area, error);
      validate();
    } else {
      setError("@raw");
      for (const [name, value] of Object.entries(parsed.value)) {
        const row = el("div", "agent-parameter");
        row.dataset.parameter = name;
        const remove = button("×", () => {
          setError(name);
          change((o) => delete o[name]);
          rebuild(current());
        }, signal, "agent-parameter-remove");
        remove.title = `Удалить ${name}`;
        remove.setAttribute("aria-label", remove.title);
        row.append(el("code", "agent-parameter-name", name), control(name, value), remove);
        body.append(row);
      }
      if (!Object.keys(parsed.value).length)
        body.append(el("p", "settings-hint", "Параметры не заданы: модуль использует свои значения по умолчанию."));
      body.append(addRow());
    }
    toggle.textContent = raw ? "Форма" : "JSON";
    count.textContent = parsed.value ? ` · ${Object.keys(parsed.value).length}` : "";
  }

  return {
    sync(state) {
      saving = state.saving;
      section.classList.toggle("busy", saving);
      const text = moduleText(state.draft, slot, module);
      if (text !== written) rebuild(text);
    },
    dispose() {
      for (const key of owned) agentSettings.setError(key);
      section.remove();
    },
  };
}
