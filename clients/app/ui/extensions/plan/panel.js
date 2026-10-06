import { node } from '../dom.js';
import { icon } from '../icons.js';
export function mount({ root, compact, hover, services }) {
  const list = node('div'); root.append(list);
  // The icon stays; progress reads as a count next to it, not as dots.
  compact.innerHTML = '<style>div{display:flex;gap:4px;align-items:center;padding:0 4px}b{font:500 11px/1 system-ui;color:var(--text-muted,#aaa);font-variant-numeric:tabular-nums}b:empty{display:none}</style>';
  const indicator = node('div'), count = node('b');
  indicator.append(icon('plan'), count); compact.append(indicator);
  let previous;
  return services['agent.session.read'].subscribe(snapshot => {
    const steps = snapshot.plan ?? [];
    const key = JSON.stringify(steps); if (key === previous) return; previous = key;
    list.replaceChildren();
    const completed = steps.filter(step => step.status === 'completed').length;
    count.textContent = steps.length ? `${completed}/${steps.length}` : '';
    compact.host.closest('button').title = `План: ${completed}/${steps.length}`;
    hover?.set(steps.length ? steps.map(step => `${step.status === 'completed' ? '✓' : step.status === 'in_progress' ? '•' : '○'} ${step.step}`).join('\n') : 'Агент ещё не составил план');
    compact.host.closest('button')?.setAttribute('aria-label', `План: ${completed}/${steps.length}`);
    if (!steps.length) list.append(node('p', 'Агент ещё не составил план', 'muted'));
    for (const step of steps) {
      const row = node('div', null, 'row'); row.style.cssText='align-items:baseline;justify-content:start;gap:8px;margin:9px 0';
      row.append(node('span', step.status === 'completed' ? '✓' : step.status === 'in_progress' ? '•' : '○'), node('span', step.step)); list.append(row);
    }
  });
}
