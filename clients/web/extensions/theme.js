export const theme = `
@import url("${new URL('../ui/select.css', import.meta.url).href}");
:host { display: block; color: var(--text-main, #ececec); font: 13px/1.5 var(--font-sans, system-ui); }
* { box-sizing: border-box; }
p { margin: 0 0 10px; }
button, textarea { font: inherit; color: inherit; border: 1px solid var(--border-strong, #444); border-radius: var(--radius-inner, 8px); }
button { padding: 5px 10px; background: var(--bg-panel-soft, #2a2a2a); cursor: pointer; }
button:disabled { opacity: .5; cursor: default; }
button:hover { background: var(--bg-panel-hover, #333); }
textarea { width: 100%; min-height: 100px; resize: vertical; padding: 10px; background: var(--bg-root, #1a1a1a); }
:focus-visible { outline: 2px solid var(--accent-blue, #6b9eff); outline-offset: 2px; }
.muted { color: var(--text-muted, #a0a0a0); }
.error { color: var(--accent-red, #ef6b6b); overflow-wrap: anywhere; }
.row { display: flex; justify-content: space-between; gap: 12px; margin-bottom: 6px; }
.row span { color: var(--text-muted, #a0a0a0); }
.row strong { font-weight: 500; text-align: right; overflow-wrap: anywhere; min-width: 0; }
details { margin: 12px 0; }
summary { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 0; cursor: pointer; list-style: none; color: var(--text-muted, #a0a0a0); }
summary::-webkit-details-marker { display: none; }
summary::after { content: '+'; flex: 0 0 auto; }
details[open] > summary::after { content: '−'; }
`;
