import { diagnosticsService } from "./diagnostics.js";
import { mountExtensionSettings } from "../../extensions/settings.js";
let preferences = {},
  composer = {},
  diagnosticUrl = "",
  writePreference,
  writeComposer,
  navigate,
  pendingModule;
const events = new EventTarget();
export function configureModules(preferenceWriter, composerWriter, onNavigate) {
  writePreference = preferenceWriter;
  writeComposer = composerWriter;
  navigate = onNavigate;
  const controller = new AbortController();
  document.addEventListener(
    "proteus-client-navigation",
    (e) => navigate(e.detail),
    { signal: controller.signal },
  );
  document.addEventListener(
    "proteus-open-settings-module",
    (e) => {
      e.preventDefault();
      requestSettingsModule(e.detail);
    },
    { signal: controller.signal },
  );
  return () => {
    controller.abort();
    writePreference = writeComposer = navigate = undefined;
  };
}
export function publishModules(preferenceJson, composerJson, url) {
  preferences = JSON.parse(preferenceJson);
  composer = JSON.parse(composerJson);
  diagnosticUrl = url;
  events.dispatchEvent(new Event("change"));
}
const service = (read, write, signal) =>
  Object.freeze({
    read: () => {
      signal.throwIfAborted();
      return structuredClone(read());
    },
    set(key, value) {
      signal.throwIfAborted();
      const error = write(key, JSON.stringify(value));
      if (error) throw Error(error);
    },
    subscribe(fn) {
      signal.throwIfAborted();
      events.addEventListener("change", fn, { signal });
      return () => events.removeEventListener("change", fn);
    },
  });
export function moduleServices(registry) {
  const services = {
    "client.preferences": (signal) =>
      service(() => preferences, writePreference, signal),
    "client.composer": (signal) =>
      service(() => composer, writeComposer, signal),
    "client.modules": (signal) =>
      Object.freeze({
        mount(root) {
          signal.throwIfAborted();
          return mountExtensionSettings(root, registry, services);
        },
      }),
    "client.diagnostics": (signal) =>
      diagnosticsService(() => diagnosticUrl, signal),
  };
  return services;
}

export function requestSettingsModule(id) {
  if (!navigate) return false;
  pendingModule = id;
  navigate(id);
  queueMicrotask(() =>
    document.dispatchEvent(
      new CustomEvent("proteus-select-settings-module", { detail: id }),
    ),
  );
  return true;
}
export function readRequestedModule() {
  const id =
    pendingModule || new URL(location.href).searchParams.get("settings_module");
  pendingModule = undefined;
  return id;
}
