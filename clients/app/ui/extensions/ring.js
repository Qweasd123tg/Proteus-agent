// Generic compact surface; the host owns interaction, the extension owns the indicator.
export function ring(root) {
  root.innerHTML = `<style>:host{display:grid;place-items:center}svg{width:28px;height:28px}circle{fill:none;stroke-width:3}.track{stroke:var(--border-strong,#444)}.value{stroke:var(--text-main,#ddd);transform:rotate(-90deg);transform-origin:16px 16px}text{fill:var(--text-main,#ddd);font:9px system-ui;text-anchor:middle}</style><svg viewBox="0 0 32 32" aria-hidden="true"><circle class="track" cx="16" cy="16" r="13"/><circle class="value" cx="16" cy="16" r="13" pathLength="100"/><text x="16" y="19">—</text></svg>`;
  const value = root.querySelector('.value'), label = root.querySelector('text');
  return (percent, title) => {
    const known = Number.isFinite(percent);
    value.setAttribute('stroke-dasharray', `${known ? Math.max(0, Math.min(100, percent)) : 0} 100`);
    label.textContent = known ? Math.round(percent) : '—';
    root.host.title = title;
    const button = root.host.closest('button');
    if (button) { button.title = title; button.setAttribute('aria-label', title); }
  };
}
