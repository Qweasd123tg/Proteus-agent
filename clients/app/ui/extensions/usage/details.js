import { mountReport } from './report.js';
import { extensionStorage } from '../storage.js';
import { theme } from '../theme.js';

export function mountUsageDetails(root, readUsage) {
  const controller = new AbortController();
  const surface = document.createElement('div'); root.replaceChildren(surface);
  const shadow = surface.attachShadow({ mode: 'open' });
  const style = document.createElement('style'); style.textContent = theme; shadow.append(style);
  const { signal } = controller;
  const stop = mountReport({ root: shadow, signal, storage: extensionStorage(localStorage, 'usage'), services: {
    'agent.usage.read': { async read() { signal.throwIfAborted(); const value = await readUsage(signal); signal.throwIfAborted(); return JSON.parse(value); } },
  } }, true);
  return () => { controller.abort(); stop(); root.replaceChildren(); };
}
