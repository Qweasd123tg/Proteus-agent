// Host-owned tabs for client views. The content stays owned by the Rust UI.
let host;
const tabs = new Map();
export function attachClientTabHost(service) {
  host = service;
  return () => {
    for (const key of [...tabs.keys()]) closeSubagentTab(key);
    if (host === service) host = undefined;
  };
}
export function openSubagentTab(key, title, content, onClose) {
  const current = tabs.get(key);
  if (current) { current.handle.show(); return false; }
  if (!host) { onClose(); return false; }
  const parking = content.parentNode;
  let item;
  const restore = () => {
    if (tabs.get(key) !== item) return;
    tabs.delete(key);
    // Restore before Rust disposes descendants or their row is removed.
    parking.append(content);
    onClose();
  };
  const handle = host.create(key, { title, onClose: restore });
  item = { handle, restore };
  tabs.set(key, item);
  handle.root.append(content);
  handle.show();
  return true;
}
export function closeSubagentTab(key) {
  const item = tabs.get(key);
  if (!item) return;
  item.handle.close();
  item.restore();
}
