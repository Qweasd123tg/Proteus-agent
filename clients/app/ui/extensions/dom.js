import { icon as createIcon } from './icons.js';

export function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text != null) element.textContent = text;
  if (className) element.className = className;
  return element;
}
export function icon(root, name) {
  const svg = createIcon(name);
  svg.setAttribute('width', '20'); svg.setAttribute('height', '20');
  root.replaceChildren(svg);
}
