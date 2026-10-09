export function createServices({ signal }) {
  const now = Math.floor(Date.now() / 1000);
  const quota = { observed_at: now, plan: 'Pro', credits: null, buckets: [{ id: 'codex', name: 'Codex', allowed: true, limit_reached: false, windows: [
    { id: 'primary', used_percent: 38, duration_seconds: 18000, resets_at: now + 8220 },
    { id: 'secondary', used_percent: 64, duration_seconds: 604800, resets_at: now + 262800 },
  ] }] };
  return { 'agent.model.quota.read': view => ({
    async read() { signal.throwIfAborted(); view.throwIfAborted(); return structuredClone(quota); },
  }) };
}
