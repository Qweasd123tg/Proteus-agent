export const style = `
  .usage-filters { display: flex; flex-wrap: wrap; gap: 8px; padding: 16px 0 10px; }
  .usage-filters input { flex: 1 1 230px; min-width: 0; padding: 7px 10px; background: var(--bg-panel-soft); border: 1px solid var(--border-subtle); border-radius: 8px; color: inherit; font: inherit; }
  .usage-filters select { flex: 0 1 auto; }
  .usage-list-heading { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 10px; margin: 4px 0 12px; }
  .usage-list-heading > span { font-size: 11px; color: var(--text-muted); }
  .usage-scope-heading { margin: 18px 0 12px; font-size: 13px; font-weight: 500; }
  .request-state { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 5px 0; }
  .request-status { padding: 1px 6px; border-radius: 5px; color: var(--text-muted); background: var(--bg-panel-soft); font-size: 10px; }
  .request-status[data-status=completed] { color: var(--accent-green); }
  .request-status[data-status=error], .request-status[data-status=timeout] { color: var(--accent-red); }
  .request-status[data-status=canceled], .request-status[data-status=unfinished] { color: var(--accent-orange, #d7aa60); }
  .request-identity > small { display: block; font-size: 10px; }
  .request-id strong { font-family: var(--font-mono, monospace); user-select: text; }
  .wide .usage-summary { padding: 18px; border: 1px solid var(--border-subtle); border-radius: 12px; background: var(--bg-panel); }
  .wide .usage-request[open] { background: var(--bg-panel); border-radius: 8px; }
  .wide .usage-request > summary { padding: 12px 8px; }
  .wide .request-metadata { padding: 4px 18px 16px; }
  @media (max-width: 600px) { .usage-filters input { flex-basis: 100%; } .usage-filters select { flex: 1 1 140px; } }
  [hidden] { display: none !important; }
  .usage-controls { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 12px; }
  select { min-width: 0; max-width: 100%; padding: 5px 8px; border: 1px solid var(--border-subtle); border-radius: 8px; background: var(--bg-panel-soft); color: inherit; font: inherit; }
  .usage-headline { display: flex; align-items: baseline; gap: 8px; margin-bottom: 14px; font-variant-numeric: tabular-nums; }
  .usage-headline > strong { font-size: 22px; font-weight: 500; }
  .usage-headline .cost-total { margin-left: auto; font-size: 17px; }
  .metric { display: flex; justify-content: space-between; align-items: baseline; gap: 16px; margin: 5px 0; }
  .metric span { min-width: 0; overflow-wrap: anywhere; }
  .metric strong { text-align: right; font-weight: 400; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; min-width: 0; }
  .metric:has(.detail-coverage) { flex-wrap: wrap; gap: 2px 16px; }
  .detail-coverage { flex-basis: 100%; text-align: right; font-size: 10px; }
  .coverage, .usage-price-note { font-size: 11px; line-height: 1.5; margin: 10px 0; }
  a { color: var(--text-main); } .usage-price-note p { margin-top: 8px; }
  .usage-price-note { margin-top: 16px; }
  .usage-models { padding: 10px 0; margin: 12px 0; border-top: 1px solid var(--border-subtle); border-bottom: 1px solid var(--border-subtle); }
  summary { cursor: pointer; }
  .usage-request { border-bottom: 1px solid var(--border-subtle); }
  .usage-request > summary { display: flex; align-items: center; gap: 10px; list-style: none; padding: 9px 0; }
  .request-identity { flex: 1; min-width: 0; }
  .request-identity strong { display: block; font-size: 12px; font-weight: 400; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .request-identity .muted { font-size: 11px; }
  .request-total { text-align: right; font-size: 12px; }
  .request-total > * { display: block; font-weight: 400; }
  .request-metadata { padding: 2px 8px 12px 14px; font-size: 12px; }
  .request-cell { text-align: right; min-width: 64px; font-variant-numeric: tabular-nums; }
  .request-cell small { display: block; color: var(--text-faint); font-size: 10px; }
  .pagination, .usage-footer { display: flex; align-items: center; gap: 8px; margin-top: 12px; }
  .usage-footer { justify-content: space-between; }
  .usage-footer span, .pagination span { font-size: 11px; }
  .wide .usage-summary { max-width: 780px; }
  .wide .usage-metrics { display: grid; grid-template-columns: 1fr 1fr; gap: 0 24px; }
  .wide .usage-request > summary { gap: 16px; }
  .wide .request-metadata { max-width: 650px; }
  @media (max-width: 850px) { .request-cell { display: none; } .wide .usage-metrics { display: block; } }
`;
