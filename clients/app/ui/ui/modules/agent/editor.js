import { agentSettings } from "./store.js";
import { moduleText, parseParameters } from "./draft.js";
import { button, el } from "./page.js";
import { fieldsForm } from "./fields.js";
import { writePath } from "./schema.js";

const pretty = value => JSON.stringify(value, null, 2);

/** Module-owned forms and an explicit JSON editor share the same sparse draft. */
export function parametersEditor(host, slot, module, signal) {
  const owned = new Set();
  const section = el("section", "agent-block agent-parameters");
  section.dataset.agentParameters = `${slot}/${module}`;
  const head = el("div", "agent-block-head"), title = el("h2", "", "Параметры");
  const toggle = button("JSON", () => { raw = !raw; rebuild(current()); }, signal, "secondary agent-mode-toggle");
  head.append(title, toggle);
  const body = el("div", "agent-parameters-body");
  section.append(head, body); host.append(section);
  let raw = false, written, saving = false, schema, schemaText, formController;
  signal.addEventListener("abort", () => formController?.abort(), { once: true });
  const current = () => moduleText(agentSettings.state().draft, slot, module);
  function setError(field, message) {
    const key = `${slot}\u001f${module}\u001f${field}`;
    if (message) owned.add(key); else owned.delete(key);
    agentSettings.setError(key, message);
  }
  function clearErrors() {
    for (const key of [...owned]) { owned.delete(key); agentSettings.setError(key); }
  }
  function write(text) {
    written = text;
    agentSettings.update(draft => {
      draft.texts[slot] ??= {};
      draft.texts[slot][module] = text;
    });
  }
  function rebuild(text) {
    formController?.abort(); formController = new AbortController();
    const formSignal = formController.signal;
    written = text; clearErrors(); body.replaceChildren();
    const parsed = parseParameters(text);
    if (parsed.error) raw = true;
    toggle.textContent = raw ? "Форма" : "JSON";
    if (raw || !schema) {
      if (!schema) body.append(el("p", "settings-hint", "Описание параметров недоступно. Настройки можно изменить в JSON."));
      const area = el("textarea", "agent-raw");
      area.spellcheck = false; area.value = text;
      area.rows = Math.min(18, Math.max(4, text.split("\n").length + 1));
      area.setAttribute("aria-label", `Параметры ${module} в JSON`);
      const error = el("p", "agent-error"); error.setAttribute("role", "status");
      const validate = () => {
        const result = parseParameters(area.value);
        error.textContent = result.error ?? ""; setError("@raw", result.error);
        toggle.disabled = saving || !schema || !!result.error;
        area.setAttribute("aria-invalid", String(!!result.error));
      };
      area.addEventListener("input", () => { write(area.value); validate(); }, { signal: formSignal });
      body.append(area, error); validate();
      return;
    }
    toggle.disabled = saving;
    if (schema.fields.length) {
      fieldsForm(body, schema.fields, {
        signal: formSignal, object: () => parseParameters(current()).value ?? {}, error: setError,
        write(path, value) {
          const object = parseParameters(current()).value ?? {};
          writePath(object, path, value); write(pretty(object));
        },
      });
    } else body.append(el("p", "settings-hint", "У этого модуля нет настраиваемых параметров."));
    const known = new Set(schema.fields.map(field => field.key));
    const unknown = Object.keys(parsed.value).filter(key => !known.has(key));
    if (unknown.length) {
      const detail = el("p", "settings-hint", `Дополнительные параметры: ${unknown.join(", ")}. Они сохраняются вместе с профилем; изменить их можно в JSON.`);
      body.append(detail);
    }
  }
  return {
    sync(state) {
      saving = state.saving; section.classList.toggle("busy", saving);
      const description = slot === "model" ? state.snapshot.model_modules.find(item => item.id === module) : slot === "hook" ? state.snapshot.hook_modules.find(item => item.id === module) :
        state.snapshot.slots.find(item => item.id === slot)?.modules.find(item => item.id === module);
      schema = description?.config_schema;
      const nextSchema = JSON.stringify(schema), text = moduleText(state.draft, slot, module);
      if (text !== written || nextSchema !== schemaText) { schemaText = nextSchema; rebuild(text); }
      section.querySelectorAll("input,select,textarea,button").forEach(control => {
        if (saving && !control.hasAttribute("data-save-disabled")) control.setAttribute("data-save-disabled", String(control.disabled));
        else if (!saving && control.hasAttribute("data-save-disabled")) {
          control.disabled = control.getAttribute("data-save-disabled") === "true";
          control.removeAttribute("data-save-disabled");
        }
        if (saving) control.disabled = true;
      });
    },
    dispose() { formController?.abort(); clearErrors(); section.remove(); },
  };
}
