import { icon } from './icons.js';
import { selectionButtons } from '../ui/modules/management.js';

// Package information is host-rendered and remains usable while its views are off.
// The switch lives in the page title row (settings-host). The preview stands in
// for the views only while they are off; an enabled page shows the real thing.
export function mountExtensionDetails(root, record, registry) {
  const controller = new AbortController(), { signal } = controller;
  let choicesController;
  const card = document.createElement('div');
  card.className = 'extension-details'; card.dataset.extensionDetails = record.id;
  const description = document.createElement('p'); description.className = 'extension-summary';
  description.textContent = record.manifest.description || 'Описание не предоставлено';
  const choices = document.createElement('div'); choices.className = 'module-selection';
  const notice = document.createElement('p'); notice.className = 'settings-status'; notice.setAttribute('role', 'status');
  const preview = document.createElement('figure'); preview.className = 'extension-preview';
  const missing = document.createElement('div'); missing.className = 'extension-preview-empty';
  const message = document.createElement('span'); message.textContent = 'Превью не предоставлено';
  missing.append(icon(record.manifest.icon || 'modules'), message);
  if (record.manifest.preview) {
    const image = document.createElement('img');
    image.src = record.manifest.preview.src; image.alt = record.manifest.preview.alt;
    image.decoding = 'async';
    image.addEventListener('error', () => preview.replaceChildren(missing), { signal });
    preview.append(image);
  } else preview.append(missing);
  card.append(description, choices, notice, preview); root.append(card);
  const unsubscribe = registry.subscribe(() => {
    const state = registry.state();
    const current = state.records.find(item => item.id === record.id);
    preview.hidden = !!current?.enabled;
    choicesController?.abort(); choicesController = new AbortController();
    choices.replaceChildren(); selectionButtons(choices, record, registry, choicesController.signal);
    choices.hidden = !choices.childElementCount;
    notice.textContent = record.source === 'package' ? state.notice || '' : '';
    notice.hidden = !notice.textContent;
  });
  return () => { controller.abort(); choicesController?.abort(); unsubscribe(); card.remove(); };
}
