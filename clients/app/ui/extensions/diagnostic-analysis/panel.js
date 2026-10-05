export function mount({ root, services }) {
  return services["client.diagnostics"].mount("analysis", root);
}
