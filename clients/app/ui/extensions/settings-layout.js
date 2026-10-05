import { node } from './dom.js';
import { watchViewMotion } from '../ui/view-motion.js';
import { watchLogicalVisibility } from '../ui/modules/visibility.js';

// Management belongs to a sidebar; the settings entry keeps the main area.
// The standalone extension host owns the same two surfaces inside its root.
export function createSettingsLayout(root, signal) {
  const page = root.closest('.settings-page');
  const sidebar = node('aside', null, 'extension-settings-sidebar extension-settings');
  sidebar.setAttribute('aria-label', 'Расширения');
  const heading = node('h2', 'Расширения', 'extension-sidebar-title');
  sidebar.append(heading);
  const empty = node('div', null, 'extension-settings-empty');
  empty.append(node('h2', 'Выберите расширение'), node('p', 'Его параметры откроются здесь. Состав и порядок расширений меняются в боковой панели.', 'settings-hint'));
  root.append(empty);
  if (page) page.insertBefore(sidebar, page.querySelector('.settings-content'));
  else {
    root.classList.add('extension-settings-layout');
    root.prepend(sidebar);
  }
  const stopMotion = watchViewMotion(sidebar, { signal });
  const stopVisibility = watchLogicalVisibility(root, visible => {
    sidebar.hidden = !visible;
    sidebar.inert = !visible;
  });
  signal.addEventListener('abort', stopVisibility, { once: true });
  return {
    sidebar,
    remove() { stopVisibility(); stopMotion(); sidebar.remove(); empty.remove(); root.classList.remove('extension-settings-layout'); },
  };
}
