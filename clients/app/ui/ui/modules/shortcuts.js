import { mountShortcutSettings } from "../shortcuts/settings.js";
export function mount({ root }) {
  root.classList.add("shortcut-settings");
  return mountShortcutSettings(root);
}
