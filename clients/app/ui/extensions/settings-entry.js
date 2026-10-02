import { createPanelRuntime } from './runtime.js';
import { extensionStorage } from './storage.js';
import { theme } from './theme.js';

// Отдельный entry настроек исполняется только при явном открытии пользователем.
export function mountSettingsEntry(root, record, storage, services = {}) {
  const surface = document.createElement('div');
  surface.className = 'extension-options-content';
  root.replaceChildren(surface);
  const shadow = surface.attachShadow({ mode: 'open' });
  const style = document.createElement('style'); style.textContent = theme; shadow.append(style);
  const runtime = createPanelRuntime({
    manifest: record.manifest.settings, root: shadow, services,
    storage: extensionStorage(storage, record.id),
    onError(error) { shadow.replaceChildren(`Не удалось открыть настройки: ${error.message}`); },
  });
  return () => { runtime.stop(); root.replaceChildren(); };
}
