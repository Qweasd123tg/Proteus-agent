/** Shared artwork for host chrome and the bundled example panels. */
const sheet = new URL('../assets/proteus-icons.svg', import.meta.url);
export function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [key, value] of Object.entries({ viewBox: '0 0 20 20', width: '18', height: '18', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.5', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true', focusable: 'false' })) svg.setAttribute(key, value);
  svg.style.cssText = 'display:block;flex:none;pointer-events:none';
  const use = document.createElementNS(svg.namespaceURI, 'use');
  use.setAttribute('href', `${sheet}#${name}`);
  svg.append(use);
  return svg;
}
