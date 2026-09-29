export function mount({ root, services, signal }) {
  return services["client.diagnostics"].mount("usage", root, signal);
}
