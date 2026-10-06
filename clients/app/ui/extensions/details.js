import { icon } from './icons.js';
import { selectionButtons } from '../ui/modules/management.js';
import { canPreview, mountLivePreview } from './live-preview.js';

// Package information is host-rendered and remains usable while its views are off.
// The switch lives in the page title row (settings-host). The preview stands in
// for the views only while they are off; an enabled page shows the real thing.
// "Попробовать" runs the views on demo services only on that explicit request.
export function mountExtensionDetails(root, record, registry) {
  const controller = new AbortController(), { signal } = controller;
  let choicesController, stopDemo;
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
  const demo = document.createElement('div'); demo.className = 'extension-demo-host';
  function closeDemo(focus) {
    if (!stopDemo) return;
    stopDemo(); stopDemo = undefined; preview.classList.remove('demo-open');
    if (focus) preview.querySelector('[data-preview-try]')?.focus({ preventScroll: true });
  }
  if (canPreview(record)) {
    const tryButton = document.createElement('button');
    tryButton.type = 'button'; tryButton.className = 'extension-preview-try'; tryButton.dataset.previewTry = record.id;
    tryButton.append(icon('play'), document.createTextNode('Попробовать'));
    tryButton.title = 'Открыть расширение на вымышленных данных; ничего не сохраняется';
    tryButton.addEventListener('click', () => {
      closeDemo(false); preview.classList.add('demo-open');
      stopDemo = mountLivePreview(demo, record, () => closeDemo(true));
    }, { signal });
    preview.append(tryButton);
  }
  root.addEventListener('module-hide', () => closeDemo(false), { signal });
  card.append(description, choices, notice, preview, demo); root.append(card);
  const unsubscribe = registry.subscribe(() => {
    const state = registry.state();
    const current = state.records.find(item => item.id === record.id);
    preview.hidden = !!current?.enabled;
    if (current?.enabled) closeDemo(false);
    choicesController?.abort(); choicesController = new AbortController();
    choices.replaceChildren(); selectionButtons(choices, record, registry, choicesController.signal);
    choices.hidden = !choices.childElementCount;
    notice.textContent = record.source === 'package' ? state.notice || '' : '';
    notice.hidden = !notice.textContent;
  });
  return () => { closeDemo(false); controller.abort(); choicesController?.abort(); unsubscribe(); card.remove(); };
}
