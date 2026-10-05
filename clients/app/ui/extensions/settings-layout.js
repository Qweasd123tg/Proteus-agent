import { node } from './dom.js';

// Management lives in the existing settings navigation.
// The standalone extension host owns the same two surfaces inside its root.
export function createSettingsLayout(root) {
  const navigation = root.closest('.settings-page')?.querySelector('.settings-extensions-nav');
  const sidebar = node('div', null, 'extension-settings-sidebar');
  const empty = node('div', null, 'extension-settings-empty');
  empty.append(node('h2', 'Выберите расширение'), node('p', 'Его параметры откроются здесь. Состав и порядок расширений меняются в боковой панели.', 'settings-hint'));
  root.append(empty);
  if (navigation) navigation.append(sidebar);
  else {
    root.classList.add('extension-settings-layout');
    sidebar.prepend(node('h2', 'Расширения'));
    root.prepend(sidebar);
  }
  return {
    sidebar,
    activate() {
      const id = root.closest('.settings-section')?.dataset.modulePage;
      if (navigation && id) document.dispatchEvent(new CustomEvent('proteus-select-settings-module', { detail: id }));
    },
    remove() { sidebar.remove(); empty.remove(); root.classList.remove('extension-settings-layout'); },
  };
}
