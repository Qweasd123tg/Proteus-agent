export function mount({ root, services }) {
  return services["client.diagnostics"].mount("architecture", root);
}
