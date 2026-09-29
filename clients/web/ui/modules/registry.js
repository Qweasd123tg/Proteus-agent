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
  let configuration = {
    enabled: Object.fromEntries(core.map((r) => [r.id, true])),
    slots: {
      "composer-model": "model-selector",
      "composer-access": "access-selector",
    },
  };
  try {
    const raw = storage.getItem(key);
    if (raw !== null) {
      const value = JSON.parse(raw);
      if (
        !value ||
        Object.keys(value).some((k) => !["enabled", "slots"].includes(k)) ||
        !value.enabled ||
        !value.slots ||
        Object.entries(value.enabled).some(
          ([id, on]) =>
            !core.some((r) => r.id === id) || typeof on !== "boolean",
        ) ||
        core.some(
          (r) =>
            !(r.id in value.enabled) || (r.required && !value.enabled[r.id]),
        ) ||
        Object.keys(value.slots).length !== slots.length ||
        slots.some(
          (s) => value.slots[s] !== null && typeof value.slots[s] !== "string",
        )
      )
        throw Error("Неверный формат модулей клиента");
      configuration = value;
    }
  } catch (error) {
    notice = `${error.message}. Восстановите встроенные модули.`;
    for (const r of core) if (!r.required) configuration.enabled[r.id] = false;
  }
  const emit = () => {
    for (const listener of listeners) listener();
  };
  const save = (next) => {
    try {
      storage.setItem(key, JSON.stringify(next));
      configuration = next;
      notice = "";
      emit();
      return true;
    } catch {
      notice = "Не удалось сохранить модули. Настройки не изменены.";
      emit();
      return false;
    }
  };
  const records = () => [
    ...core.map((r) => ((r.enabled = configuration.enabled[r.id]), r)),
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
    resetCore: () =>
      save({
        enabled: Object.fromEntries(core.map((r) => [r.id, true])),
        slots: {
          "composer-model": "model-selector",
          "composer-access": "access-selector",
        },
      }),
    dispose() {
      unsubscribe();
      listeners.clear();
    },
  };
}
