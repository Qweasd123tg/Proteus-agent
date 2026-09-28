import { node } from '../dom.js';
import { icon } from '../icons.js';
export function mount({ root, compact, hover, services }) {
  const list = node('div'); root.append(list);
  compact.innerHTML = '<style>:host{display:flex;gap:2px;align-items:center;flex-wrap:wrap;max-width:26px}i{width:5px;height:5px;border-radius:50%;background:var(--border-strong,#555)}.completed{background:var(--text-main,#ddd)}.in_progress{background:var(--accent-blue,#79a9ff)}</style>';
  compact.append(icon('plan'));
  let previous;
  return services['agent.session.read'].subscribe(snapshot => {
    const steps = snapshot.plan ?? [];
    const key = JSON.stringify(steps); if (key === previous) return; previous = key;
    list.replaceChildren(); compact.querySelectorAll('svg,i').forEach(item => item.remove());
    const completed = steps.filter(step => step.status === 'completed').length;
    compact.host.closest('button').title = `План: ${completed}/${steps.length}`;
    hover?.set(steps.length ? steps.map(step => `${step.status === 'completed' ? '✓' : step.status === 'in_progress' ? '•' : '○'} ${step.step}`).join('\n') : 'Агент ещё не составил план');
    compact.host.closest('button')?.setAttribute('aria-label', `План: ${completed}/${steps.length}`);
    if (!steps.length) { list.append(node('p', 'Агент ещё не составил план', 'muted')); compact.append(icon('plan')); }
    for (const step of steps) {
      const row = node('div', null, 'row'); row.style.cssText='align-items:baseline;justify-content:start;gap:8px;margin:9px 0';
      row.append(node('span', step.status === 'completed' ? '✓' : step.status === 'in_progress' ? '•' : '○'), node('span', step.step)); list.append(row);
      const dot = node('i', null, step.status); compact.append(dot);
    }
  });
}
