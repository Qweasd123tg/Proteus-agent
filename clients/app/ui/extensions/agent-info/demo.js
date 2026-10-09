export function createServices({ signal }) {
  const config = { profile: 'demo', registered_tools: ['read_file', 'list_dir', 'grep', 'apply_patch', 'shell', 'update_plan'].map(name => ({ name })) };
  return { 'agent.config.read': view => ({
    async read() { signal.throwIfAborted(); view.throwIfAborted(); return structuredClone(config); },
  }) };
}
