import { icon } from '../dom.js';
export function mount({ root, compact, storage, signal }) {
  icon(compact, 'M4 4h10M4 4v16h16V10M10 14l1-4 8-8 3 3-8 8Z');
  const textarea = document.createElement('textarea');
  textarea.setAttribute('aria-label', 'Личные заметки');
  textarea.placeholder = 'Идеи, ссылки, что проверить…';
  const status = document.createElement('p');
  status.className = 'muted';
  status.setAttribute('role', 'status');
  textarea.value = storage.get('text') ?? '';
  status.textContent = 'Сохраняются в этом клиенте';
  textarea.addEventListener('input', () => {
    try {
      storage.set('text', textarea.value);
      status.className = 'muted';
      status.textContent = 'Сохранено';
    } catch {
      status.className = 'error';
      status.textContent = 'Не удалось сохранить заметку';
    }
  }, { signal });
  root.append(textarea, status);
}
