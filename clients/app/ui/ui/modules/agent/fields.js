import { button, el } from "./page.js";
import { initialValue, parseScalar, readPath, sameValue, valueError, writePath } from "./schema.js";

let serial = 0;

/** Reusable typed controls. The caller owns sparse values and the save draft. */
export function fieldsForm(host, fields, context, base = [], inherited = {}) {
  const regular = el("div", "agent-fields"), advanced = el("details", "agent-advanced");
  const extra = el("div", "agent-fields");
  advanced.append(el("summary", "", "Дополнительно"), extra);
  let extraCount = 0;
  for (const field of fields) {
    const path = [...base, field.key];
    const row = el("div", `agent-field${field.value.type === "object" ? " agent-field-group" : ""}`);
    row.dataset.parameter = path.join(".");
    const id = `agent-field-${++serial}`;
    const label = el("label", "settings-label");
    label.htmlFor = id;
    const title = el("strong", "", field.title);
    title.title = path.join(".");
    const hint = el("span", "settings-hint", field.description);
    hint.id = `${id}-hint`;
    label.append(title, hint);
    const wrap = el("div", "agent-field-control"), inputHost = el("div", "agent-field-input");
    const meta = el("span", "agent-field-origin"), error = el("p", "agent-error");
    error.id = `${id}-error`;
    error.setAttribute("role", "status");
    const specified = () => readPath(context.object(), path);
    const defaultValue = field.default ?? inherited?.[field.key];
    const effective = () => specified() !== undefined ? specified() : defaultValue;
    const reset = button("↶", () => {
      context.write(path, undefined);
      rebuild();
    }, context.signal, "agent-field-reset");
    reset.title = `Сбросить «${field.title}»`;
    reset.setAttribute("aria-label", reset.title);
    function report(message) {
      error.textContent = message || "";
      row.toggleAttribute("data-invalid", !!message);
      inputHost.querySelectorAll("input,select,textarea").forEach(input => input.setAttribute("aria-invalid", String(!!message)));
      context.error(path.join("."), message);
    }
    function refresh() {
      const explicit = specified() !== undefined;
      meta.textContent = explicit ? "Своё значение" : defaultValue != null ? "По умолчанию" : field.required ? "Нужно задать" : "Не задано";
      reset.disabled = !explicit;
      row.classList.toggle("overridden", explicit);
    }
    function rebuild() {
      inputHost.replaceChildren();
      const current = effective();
      if (field.value.type === "object") {
        const details = el("details", "agent-object");
        details.append(el("summary", "", "Показать параметры"));
        details.open = specified() !== undefined;
        fieldsForm(details, field.value.fields, {
          ...context,
          active: () => context.active?.() !== false && (specified() !== undefined || field.required),
          write(childPath, value) {
            context.write(childPath, value);
            refresh();
            report(specified() === undefined ? null : valueError(field.value, specified()));
          },
        }, path, current || {});
        inputHost.append(details);
      } else {
        const control = valueControl(field.value, current, {
          signal: context.signal,
          write(value) { context.write(path, value); refresh(); report(valueError(field.value, value)); },
          error: report,
        });
        const main = control.matches("input,select,textarea") ? control : control.querySelector("input,select,textarea");
        if (main) {
          main.id = id;
          main.setAttribute("aria-label", field.title);
          main.setAttribute("aria-describedby", `${id}-hint ${id}-error`);
        }
        inputHost.append(control);
        if (field.unit) inputHost.append(el("span", "agent-field-unit", field.unit));
      }
      refresh();
      report(current === undefined ? (field.required && context.active?.() !== false ? "Задайте значение" : null) : valueError(field.value, current));
    }
    wrap.append(inputHost, meta);
    row.append(label, wrap, reset, error);
    (field.advanced ? extra : regular).append(row);
    if (field.advanced) extraCount++;
    rebuild();
  }
  host.append(regular);
  if (extraCount) host.append(advanced);
}

function valueControl(schema, value, context) {
  if (schema.type === "array") return arrayControl(schema.items, Array.isArray(value) ? value : [], context);
  if (schema.type === "object") {
    const host = el("div", "agent-array-object");
    let object = value && typeof value === "object" && !Array.isArray(value) ? structuredClone(value) : {};
    const errors = new Map();
    fieldsForm(host, schema.fields, {
      ...context, object: () => object,
      write(path, next) {
        // Array item objects are owned locally; no default siblings are materialized.
        writePath(object, path, next);
        context.write(structuredClone(object));
      },
      error: (key, message) => {
        if (message) errors.set(key, message); else errors.delete(key);
        context.error([...errors.values()][0] || valueError(schema, object));
      },
    });
    return host;
  }
  if (schema.type === "enum") {
    const select = el("select");
    const blank = new Option("Не задано", "");
    blank.disabled = true;
    select.add(blank);
    schema.options.forEach((option, index) => select.add(new Option(option.title, String(index))));
    const index = schema.options.findIndex(option => sameValue(option.value, value));
    if (value !== undefined && index < 0) {
      const unknown = new Option(`Неизвестное значение: ${JSON.stringify(value)}`, "unknown");
      unknown.disabled = true; select.add(unknown); select.value = "unknown";
    } else select.value = index < 0 ? "" : String(index);
    select.addEventListener("change", () => context.write(schema.options[Number(select.value)].value), { signal: context.signal });
    return select;
  }
  const input = el(schema.type === "json" || schema.type === "string" && schema.multiline ? "textarea" : "input");
  if (schema.type === "boolean") {
    input.type = "checkbox"; input.className = "settings-toggle";
    input.checked = value === true;
    input.addEventListener("change", () => context.write(input.checked), { signal: context.signal });
    return input;
  }
  if (input.tagName === "INPUT") {
    input.type = schema.secret ? "password" : "text";
    if (["integer", "number"].includes(schema.type)) input.inputMode = schema.type === "integer" ? "numeric" : "decimal";
  } else { input.rows = 3; input.className = "agent-json-value"; }
  input.spellcheck = false;
  input.autocomplete = "off";
  input.value = value === undefined ? "" : schema.type === "json" ? JSON.stringify(value, null, 2) : typeof value === "string" ? value : String(value);
  input.placeholder = "Не задано";
  input.addEventListener("input", () => {
    const result = parseScalar(schema, input.value);
    if (result.error) context.error(result.error);
    else context.write(result.value);
  }, { signal: context.signal });
  return input;
}

function arrayControl(items, value, context) {
  const host = el("div", "agent-array"), list = el("div", "agent-array-items");
  let values = structuredClone(value);
  let renderController;
  const errors = new Map();
  const report = () => {
    add.disabled = errors.size > 0;
    host.querySelectorAll("[data-array-remove]").forEach(button => {
      button.disabled = [...errors.keys()].some(index => index !== Number(button.dataset.arrayRemove));
    });
    context.error([...errors.values()][0] || valueError({ type: "array", items }, values));
  };
  context.signal.addEventListener("abort", () => renderController?.abort(), { once: true });
  function render() {
    renderController?.abort();
    renderController = new AbortController();
    const signal = renderController.signal;
    list.replaceChildren();
    if (!values.length) list.append(el("span", "settings-hint", "Список пуст"));
    values.forEach((item, index) => {
      const row = el("div", "agent-array-item");
      const control = valueControl(items, item, {
        ...context,
        signal,
        write(next) { errors.delete(index); values[index] = next; context.write(structuredClone(values)); report(); },
        error(message) { if (message) errors.set(index, message); else errors.delete(index); report(); },
      });
      control.setAttribute("aria-label", `Элемент ${index + 1}`);
      const remove = button("×", () => {
        if (errors.size && !errors.has(index)) return;
        values.splice(index, 1); errors.clear(); context.write(structuredClone(values)); render(); report();
      }, signal, "agent-field-reset");
      remove.setAttribute("aria-label", `Удалить элемент ${index + 1}`);
      remove.dataset.arrayRemove = index;
      row.append(control, remove);
      list.append(row);
    });
    report();
  }
  const add = button("Добавить", () => {
    if (errors.size) return;
    values.push(initialValue(items)); context.write(structuredClone(values)); render();
    list.lastElementChild?.querySelector("input,select,textarea")?.focus();
  }, context.signal, "secondary agent-array-add");
  host.append(list, add); render();
  return host;
}
