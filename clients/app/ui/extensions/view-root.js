import { theme } from './theme.js';

// DOM isolation is view metadata, independent of the package source.
export function createViewRoot(container, isolation, className) {
  const element = document.createElement('div');
  element.className = className;
  container.append(element);
  const root = isolation === 'shadow' ? element.attachShadow({ mode: 'open' }) : element;
  if (isolation === 'shadow') {
    const style = document.createElement('style');
    style.textContent = theme;
    root.append(style);
  }
  return { element, root };
}
