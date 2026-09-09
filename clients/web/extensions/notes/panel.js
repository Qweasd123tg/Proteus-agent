export function mount({ root, storage, signal }) {
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
