// Installed before the Leptos entrypoint on every native client navigation.
document.addEventListener('click', (event) => {
  const link = event.target.closest?.('a');
  if (!link) return;
  const href = link.getAttribute('href') || '';
  const route = href.split('?', 1)[0];
  const target = route === 'proteus-desktop:inspector' ? 'inspector' : route === 'proteus-desktop:chat' ? 'main' : null;
  if (target) {
    event.preventDefault();
    const query = href.includes('?') ? href.slice(href.indexOf('?') + 1) : '';
    const sessionDir = new URLSearchParams(query).get('session_dir');
    window.__TAURI__.core.invoke('open_client', { label: target, sessionDir }).catch(error => console.error(error));
  }
}, true);
