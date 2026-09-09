// Installed before the Leptos entrypoint on every native client navigation.
document.addEventListener('click', (event) => {
  const link = event.target.closest?.('a');
  if (!link) return;
  const href = link.getAttribute('href') || '';
  const target = href === 'proteus-desktop:inspector' ? 'inspector' : href === 'proteus-desktop:chat' ? 'main' : null;
  if (target) {
    event.preventDefault();
    window.__TAURI__.core.invoke('open_client', { label: target }).catch(error => console.error(error));
  }
}, true);
