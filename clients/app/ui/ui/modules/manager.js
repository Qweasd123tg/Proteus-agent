export function mount({ root, services }) {
  root.classList.add("extension-settings");
  return services["client.modules"].mount(root);
}
