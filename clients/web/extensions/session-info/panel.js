import { node } from '../dom.js';
export function mount({ root, compact, services }) {
  compact.textContent = 'ⓘ';
  const fields = [['status','Статус'],['model','Модель'],['mode','Режим'],['reasoning','Reasoning'],['workspace','Проект'],['events','Событий'],['tools','Инструментов'],['pending','Ожидают']];
  const values = new Map();
  for (const [key, label] of fields) {
    const row = node('div', null, 'row'); const value = node('code');
    value.style.cssText = 'text-align:right;overflow-wrap:anywhere;min-width:0';
    row.append(node('span', label, 'muted'), value); root.append(row); values.set(key,value);
  }
  return services['agent.session.read'].subscribe(snapshot => {
    for (const [key, value] of values) { const text = String(snapshot[key] ?? '—'); if (value.textContent !== text) value.textContent = text; }
    compact.host.title = `Сессия: ${snapshot.status ?? '—'}`;
  });
}
