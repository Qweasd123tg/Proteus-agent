export function mount({ root, services, signal }) {
  return services["client.diagnostics"].mount("analysis", root, signal);
}
