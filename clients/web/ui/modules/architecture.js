export function mount({ root, services, signal }) {
  return services["client.diagnostics"].mount("architecture", root, signal);
}
