import { icon } from './icons.js';
import { selectionButtons } from '../ui/modules/management.js';

// Package information is host-rendered and remains usable while its views are off.
export function mountExtensionDetails(root, record, registry) {
  const controller = new AbortController(), { signal } = controller;
  let choicesController;
  const card = document.createElement('div');
  card.className = 'extension-details'; card.dataset.extensionDetails = record.id;
  const about = document.createElement('div'); about.className = 'extension-about';
  const row = document.createElement('div'); row.className = 'extension-enable-row';
  if (record.source === 'builtin') row.dataset.builtinModule = record.id;
  const label = document.createElement('label'); label.className = 'extension-enable';
  const input = document.createElement('input');
  input.type = 'checkbox'; input.className = 'settings-toggle'; input.dataset.extensionToggle = record.id;
  input.setAttribute('aria-label', 'Включить: ' + record.manifest.name);
  const status = document.createElement('span');
  label.append(input, status); row.append(label);
  input.addEventListener('change', () => registry.update(record.id, { enabled: input.checked }), { signal });
  const description = document.createElement('p'); description.className = 'extension-summary';
  description.textContent = record.manifest.description || 'Описание не предоставлено';
  const required = document.createElement('p'); required.className = 'settings-hint';
  required.textContent = 'Обязательная часть приложения'; required.hidden = !record.required;
  const choices = document.createElement('div'); choices.className = 'module-selection';
  const notice = document.createElement('p'); notice.className = 'settings-status'; notice.setAttribute('role', 'status');
  row.append(required, choices, notice);
  about.append(description);
  const preview = document.createElement('figure'); preview.className = 'extension-preview';
  const missing = document.createElement('div'); missing.className = 'extension-preview-empty';
  const message = document.createElement('span'); message.textContent = 'Превью не предоставлено';
  missing.append(icon(record.manifest.icon || 'modules'), message);
  if (record.manifest.preview) {
    const image = document.createElement('img');
    image.src = record.manifest.preview.src; image.alt = record.manifest.preview.alt;
    image.loading = 'lazy'; image.decoding = 'async';
    image.addEventListener('error', () => preview.replaceChildren(missing), { signal });
    preview.append(image);
  } else preview.append(missing);
  const information = document.createElement('div'); information.className = 'extension-info';
  information.append(about, preview);
  card.append(row, information); root.append(card);
  const unsubscribe = registry.subscribe(() => {
    const state = registry.state();
    const current = state.records.find(item => item.id === record.id);
    input.checked = !!current?.enabled;
    input.disabled = !!record.required || !current || (record.source === 'package' && (state.busy || !state.ready));
    status.textContent = input.checked ? 'Включено' : 'Выключено';
    choicesController?.abort(); choicesController = new AbortController();
    choices.replaceChildren(); selectionButtons(choices, record, registry, choicesController.signal);
    notice.textContent = record.source === 'package' ? state.notice || '' : '';
    notice.hidden = !notice.textContent;
  });
  return () => { controller.abort(); choicesController?.abort(); unsubscribe(); card.remove(); };
}
