import { parseManifest, hasSurface } from "../../extensions/contract.js";
import { builtins } from "./catalog.js";
const key = "proteus.ui.modules";
const slots = ["composer-model", "composer-access"];
export function createClientModuleRegistry(
  packages,
  storage = packages.storage,
  catalog = builtins,
) {
  const listeners = new Set();
  let notice = "";
  const core = catalog.map((record) => ({
    ...record,
    manifest: parseManifest(record.manifest, record.manifest.entry),
  }));
  const defaults = () => ({
    enabled: Object.fromEntries(core.map((r) => [r.id, true])),
    slots: {
      "composer-model": "model-selector",
      "composer-access": "access-selector",
    },
  });
  // Only the user's choices are stored, so built-in pages added or removed by
  // an update leave the rest of the selection intact.
  const parse = (value) => {
    if (
      !value ||
      typeof value !== "object" ||
      Object.keys(value).some((k) => !["disabled", "slots"].includes(k)) ||
      !Array.isArray(value.disabled) ||
      value.disabled.some((id) => typeof id !== "string") ||
      new Set(value.disabled).size !== value.disabled.length ||
      !value.slots ||
      Object.keys(value.slots).length !== slots.length ||
      slots.some(
        (s) => value.slots[s] !== null && typeof value.slots[s] !== "string",
      )
    )
      throw Error("Неверный формат расширений интерфейса");
    const next = defaults();
    for (const id of value.disabled) {
      const record = core.find((r) => r.id === id);
      if (!record)
        throw Error(`Выключенного встроенного расширения «${id}» больше нет`);
      if (record.required)
        throw Error(`Расширение «${record.manifest.name}» нельзя выключить`);
      next.enabled[id] = false;
    }
    next.slots = { ...value.slots };
    return next;
  };
  const serialize = (next) =>
    JSON.stringify({
      disabled: core
        .filter((r) => !r.required && next.enabled[r.id] === false)
        .map((r) => r.id),
      slots: next.slots,
    });
  let configuration = defaults();
  let invalid = false;
  try {
    const raw = storage.getItem(key);
    if (raw !== null) configuration = parse(JSON.parse(raw));
  } catch (error) {
    invalid = true;
    notice = `${error.message}. Восстановите встроенные расширения.`;
    for (const r of core) if (!r.required) configuration.enabled[r.id] = false;
  }
  const emit = () => {
    for (const listener of listeners) listener();
  };
  const save = (next) => {
    try {
      storage.setItem(key, serialize(next));
      configuration = next;
      notice = "";
      invalid = false;
      emit();
      return true;
    } catch {
      notice = "Не удалось сохранить расширения. Настройки не изменены.";
      emit();
      return false;
    }
  };
  const records = () => [
    ...core.map(
      (r) => ((r.enabled = r.required || configuration.enabled[r.id]), r),
    ),
    ...packages.state().records,
  ];
  const unsubscribe = packages.subscribe(emit);
  return {
    storage,
    state() {
      const state = packages.state();
      return {
        ...state,
        records: records(),
        notice: [notice, state.notice].filter(Boolean).join(" "),
        builtinsInvalid: invalid,
        slots: { ...configuration.slots },
      };
    },
    start: () => packages.start(),
    subscribe(fn) {
      listeners.add(fn);
      fn();
      return () => listeners.delete(fn);
    },
    update(id, change) {
      const r = core.find((r) => r.id === id);
      if (!r) return packages.update(id, change);
      if (r.required || typeof change.enabled !== "boolean") return;
      save({
        ...configuration,
        enabled: { ...configuration.enabled, [id]: change.enabled },
      });
    },
    select(slot, id) {
      const r = records().find((r) => r.id === id);
      if (!slots.includes(slot) || !r?.enabled || !hasSurface(r.manifest, slot))
        return false;
      return save({
        ...configuration,
        slots: { ...configuration.slots, [slot]: id },
      });
    },
    selected(slot) {
      const id = configuration.slots[slot];
      return records().find(
        (r) => r.id === id && r.enabled && hasSurface(r.manifest, slot),
      );
    },
    move: (id, step) => packages.move(id, step),
    remove: (id) => {
      if (!core.some((r) => r.id === id)) packages.remove(id);
    },
    addBundled: (id) => packages.addBundled(id),
    install: (url) => packages.install(url),
    reset: () => packages.reset(),
    resetCore: () => save(defaults()),
    dispose() {
      unsubscribe();
      listeners.clear();
    },
  };
}
