import { selectionButtons } from '../ui/modules/management.js';
import { canPreview, mountLivePreview } from './live-preview.js';
import { observeVisibility } from './visibility.js';

// Package information is host-rendered and remains usable while its views are off.
// A visible page always previews the package with demo services, independently
// of its enabled state. Hidden pages release demos, not retained real settings.
export function mountExtensionDetails(root, record, registry) {
  const controller = new AbortController(), { signal } = controller;
  let choicesController, stopDemo;
  const card = document.createElement('div');
  card.className = 'extension-details'; card.dataset.extensionDetails = record.id;
  const description = document.createElement('p'); description.className = 'extension-summary';
  description.textContent = record.manifest.description || 'Описание не предоставлено';
  const choices = document.createElement('div'); choices.className = 'module-selection';
  const notice = document.createElement('p'); notice.className = 'settings-status'; notice.setAttribute('role', 'status');
  const previewable = canPreview(record);
  const demo = document.createElement('div'); demo.className = 'extension-demo-host';
  function closeDemo() {
    if (!stopDemo) return;
    stopDemo(); stopDemo = undefined;
  }
  root.addEventListener('module-hide', closeDemo, { signal });
  card.append(description, choices, notice, demo); root.append(card);
  observeVisibility(root, visible => {
    if (!visible) closeDemo();
    else if (previewable && !stopDemo) stopDemo = mountLivePreview(demo, record);
  }, signal);
  const unsubscribe = registry.subscribe(() => {
    const state = registry.state();
    choicesController?.abort(); choicesController = new AbortController();
    choices.replaceChildren(); selectionButtons(choices, record, registry, choicesController.signal);
    choices.hidden = !choices.childElementCount;
    notice.textContent = record.source === 'package' ? state.notice || '' : '';
    notice.hidden = !notice.textContent;
  });
  return () => { closeDemo(); controller.abort(); choicesController?.abort(); unsubscribe(); card.remove(); };
}
