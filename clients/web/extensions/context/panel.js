import { node } from '../dom.js';
import { ring } from '../ring.js';
export function mount({ root, compact, services }) {
  const updateRing = ring(compact);
  const label = node('p'), bar = node('progress'), threshold = node('p', '', 'muted');
  bar.max = 100; bar.style.width='100%'; bar.setAttribute('aria-label','Заполнение контекста');
  root.append(label,bar,threshold);
  let previous;
  return services['agent.session.read'].subscribe(snapshot => {
    const usage = snapshot.context, key = JSON.stringify(usage); if (key === previous) return; previous = key;
    const percent = usage?.max ? 100 * usage.used / usage.max : null;
    label.textContent = usage ? `${usage.used.toLocaleString()} / ${usage.max.toLocaleString()} токенов` : 'Замеров ещё нет';
    bar.value = percent ?? 0; bar.hidden = !usage;
    threshold.textContent = usage?.trigger ? `Автокомпакт: ${usage.trigger.toLocaleString()} токенов` : '';
    updateRing(percent, usage ? `Контекст: ${Math.round(percent ?? 0)}%` : 'Контекст: нет данных');
  });
}
