export function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text != null) element.textContent = text;
  if (className) element.className = className;
  return element;
}
export function icon(root, path) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox','0 0 24 24'); svg.setAttribute('fill','none');
  svg.setAttribute('stroke','currentColor'); svg.setAttribute('stroke-width','1.5');
  svg.style.cssText='width:24px;height:24px';
  const shape = document.createElementNS(svg.namespaceURI,'path'); shape.setAttribute('d',path); svg.append(shape);
  root.replaceChildren(svg);
}
