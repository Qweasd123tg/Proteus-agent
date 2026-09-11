import { graphModel, layoutGraph, relatedNodes, matches, edgeLabel, NODE_WIDTH, NODE_HEIGHT } from './model.js';
import { element, renderDetails } from './details.js';

const SVG = 'http://www.w3.org/2000/svg';
let instance = 0;
function svg(tag, attributes = {}) {
  const node = document.createElementNS(SVG, tag);
  for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
  return node;
}

export function mountTopologyGraph(root, source) {
  const model = graphModel(JSON.parse(source));
  const controller = new AbortController();
  const { signal } = controller;
  let scope = 'assembly', inactive = false, selected = null, query = '';
  let layout, scale = 1, offset = { x: 0, y: 0 }, drag, autoFit = true;
  const nodeElements = new Map(), edgeElements = [];
  root.classList.add('topology-explorer');
  const toolbar = element('div', null, 'graph-toolbar');
  const scopes = element('div', null, 'graph-scopes');
  scopes.setAttribute('role', 'group'); scopes.setAttribute('aria-label', 'Состав карты');
  function button(parent, label, action, className) {
    const button = element('button', label, className);
    button.type = 'button'; button.addEventListener('click', action, { signal }); parent.append(button);
    return button;
  }
  for (const [id, label] of [['assembly', 'Сборка'], ['modules', 'Модули'], ['tools', 'Инструменты']]) {
    const item = button(scopes, label, () => { scope = id; selected = null; draw(); });
    item.dataset.scope = id;
  }
  const search = element('input');
  search.type = 'search'; search.placeholder = 'Найти на карте…'; search.setAttribute('aria-label', 'Поиск объектов карты');
  const inactiveLabel = element('label', null, 'graph-inactive');
  const checkbox = element('input'); checkbox.type = 'checkbox';
  checkbox.addEventListener('change', () => { inactive = checkbox.checked; selected = null; draw(); }, { signal });
  inactiveLabel.append(checkbox, document.createTextNode('Неактивные'));
  toolbar.append(scopes, search, inactiveLabel);
  const workspace = element('div', null, 'graph-workspace');
  const viewport = element('div', null, 'graph-viewport');
  viewport.tabIndex = 0; viewport.setAttribute('role', 'region'); viewport.setAttribute('aria-label', 'Карта связей: перемещение мышью или стрелками, масштаб плюс и минус');
  const stage = element('div', null, 'graph-stage');
  stage.style.setProperty('--graph-node-width', `${NODE_WIDTH}px`);
  stage.style.setProperty('--graph-node-height', `${NODE_HEIGHT}px`);
  const connections = svg('svg', { class: 'graph-connections', 'aria-hidden': 'true' });
  const markerId = `graph-arrow-${++instance}`;
  const controls = element('div', null, 'graph-controls');
  const zoomLabel = element('span', '', 'graph-zoom');
  const details = element('aside', null, 'graph-details');
  details.hidden = true;
  details.setAttribute('aria-label', 'Свойства выбранного объекта');
  const searchResults = element('div', null, 'graph-search-results'); searchResults.hidden = true;
  const status = element('p', '', 'graph-status'); status.setAttribute('role', 'status');
  const footer = element('div', null, 'graph-footer');
  footer.append(status, element('span', 'Перетаскивание · колесо — масштаб · Esc — сброс', 'graph-help'));
  viewport.append(stage, controls); workspace.append(viewport, details);
  root.append(toolbar, searchResults, workspace, footer);
  button(controls, '−', () => zoom(1 / 1.2), 'graph-zoom-out').setAttribute('aria-label', 'Уменьшить карту');
  controls.append(zoomLabel);
  button(controls, '+', () => zoom(1.2), 'graph-zoom-in').setAttribute('aria-label', 'Увеличить карту');
  button(controls, 'Вписать', fit);
  const expand = button(controls, 'Развернуть', () => {
    root.classList.toggle('fullscreen');
    expand.textContent = root.classList.contains('fullscreen') ? 'Свернуть' : 'Развернуть';
    expand.setAttribute('aria-expanded', String(root.classList.contains('fullscreen')));
    autoFit = true; requestAnimationFrame(fit);
  });
  expand.setAttribute('aria-expanded', 'false');

  function transform() {
    stage.style.transform = `translate(${offset.x}px, ${offset.y}px) scale(${scale})`;
    zoomLabel.textContent = `${Math.round(scale * 100)}%`;
  }
  function fit() {
    if (!layout || !viewport.clientWidth || !viewport.clientHeight) return;
    scale = Math.max(.12, Math.min(1, (viewport.clientWidth - 40) / layout.width, (viewport.clientHeight - 40) / layout.height));
    offset = { x: (viewport.clientWidth - layout.width * scale) / 2, y: (viewport.clientHeight - layout.height * scale) / 2 };
    autoFit = true; transform();
  }
  function zoom(factor, x = viewport.clientWidth / 2, y = viewport.clientHeight / 2) {
    const next = Math.max(.12, Math.min(2.5, scale * factor));
    offset = { x: x - (x - offset.x) * next / scale, y: y - (y - offset.y) * next / scale };
    scale = next; autoFit = false; transform();
  }
  function center(id) {
    const node = layout.nodes.find(node => node.id === id);
    if (!node) return;
    scale = Math.max(.8, scale);
    offset = { x: viewport.clientWidth / 2 - (node.x + NODE_WIDTH / 2) * scale, y: viewport.clientHeight / 2 - (node.y + NODE_HEIGHT / 2) * scale };
    autoFit = false; transform();
  }
  function select(id, focus = true) {
    selected = id;
    if (!layout.nodes.some(node => node.id === id)) {
      const node = model.nodes.get(id);
      scope = node.kind === 'module' ? 'modules' : node.kind === 'tool' ? 'tools' : 'assembly';
      if (node.active === false) inactive = true;
      draw();
    } else highlight();
    if (focus) center(id);
  }
  function highlight() {
    details.hidden = !selected;
    const related = selected ? relatedNodes(model, selected) : null;
    for (const [id, button] of nodeElements) {
      button.classList.toggle('selected', selected === id);
      button.classList.toggle('dimmed', !!related && !related.has(id));
      button.classList.toggle('search-match', !!query && matches(model.nodes.get(id), query));
      button.setAttribute('aria-pressed', String(selected === id));
    }
    for (const [edge, path] of edgeElements) {
      path.classList.toggle('selected', edge.from === selected || edge.to === selected);
      path.classList.toggle('dimmed', !!selected && edge.from !== selected && edge.to !== selected);
    }
    renderDetails(details, model, selected, id => {
      select(id);
      const heading = details.querySelector('h3');
      heading.tabIndex = -1; heading.focus({ preventScroll: true });
    });
    status.textContent = selected ? `Выбрано: ${model.nodes.get(selected).label}` : `Выберите объект · ${layout.nodes.length} объектов · ${layout.edges.length} связей`;
  }
  function results() {
    query = search.value.trim(); searchResults.replaceChildren(); searchResults.hidden = !query;
    if (query) {
      const found = [...model.nodes.values()].filter(node => matches(node, query));
      searchResults.append(element('span', found.length ? `Найдено: ${found.length}` : 'Ничего не найдено', 'graph-muted'));
      for (const node of found) {
        const result = element('button', `${node.label} · ${node.subtitle}`);
        result.type = 'button'; result.addEventListener('click', () => select(node.id));
        searchResults.append(result);
      }
    }
    highlight();
  }
  search.addEventListener('input', results, { signal });

  function draw() {
    layout = layoutGraph(model, scope, inactive);
    nodeElements.clear(); edgeElements.length = 0;
    stage.replaceChildren(connections); connections.replaceChildren();
    stage.style.width = `${layout.width}px`; stage.style.height = `${layout.height}px`;
    connections.setAttribute('width', layout.width); connections.setAttribute('height', layout.height);
    const marker = svg('marker', { id: markerId, viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 5, markerHeight: 5, orient: 'auto-start-reverse' });
    marker.append(svg('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill: 'context-stroke' }));
    const defs = svg('defs'); defs.append(marker); connections.append(defs);
    const positions = new Map(layout.nodes.map(node => [node.id, node]));
    for (const edge of layout.edges) {
      const from = positions.get(edge.from), to = positions.get(edge.to);
      const forward = to.x >= from.x;
      const x1 = from.x + (forward ? NODE_WIDTH : 0), x2 = to.x + (forward ? 0 : NODE_WIDTH);
      const y1 = from.y + NODE_HEIGHT / 2, y2 = to.y + NODE_HEIGHT / 2;
      const bend = Math.max(44, Math.abs(x2 - x1) / 2) * (forward ? 1 : -1);
      const curve = from.x === to.x
        ? `M${x1},${y1} C${x1 + 64},${y1} ${x1 + 64},${y2} ${x1},${y2}`
        : `M${x1},${y1} C${x1 + bend},${y1} ${x2 - bend},${y2} ${x2},${y2}`;
      const path = svg('path', { d: curve, 'marker-end': `url(#${markerId})`, 'data-edge-kind': edge.kind });
      const title = svg('title'); title.textContent = edgeLabel(edge); path.append(title);
      connections.append(path); edgeElements.push([edge, path]);
    }
    for (const node of layout.nodes) {
      const item = element('button', null, 'graph-node'); item.type = 'button';
      item.dataset.nodeId = node.id; item.dataset.kind = node.kind;
      item.classList.toggle('missing', !!node.missing);
      item.style.left = `${node.x}px`; item.style.top = `${node.y}px`;
      item.title = `${node.label}\n${node.subtitle}`;
      item.append(element('strong', node.label), element('span', node.subtitle));
      item.addEventListener('click', () => select(node.id, false));
      item.addEventListener('focus', () => { if (item.matches(':focus-visible')) center(node.id); });
      stage.append(item); nodeElements.set(node.id, item);
    }
    for (const item of scopes.children) item.setAttribute('aria-pressed', String(item.dataset.scope === scope));
    inactiveLabel.hidden = scope === 'assembly'; checkbox.checked = inactive;
    highlight(); fit();
  }
  viewport.addEventListener('wheel', event => {
    if (event.target.closest('.graph-controls')) return;
    event.preventDefault();
    const rect = viewport.getBoundingClientRect();
    const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? viewport.clientHeight : 1);
    zoom(Math.exp(-Math.max(-120, Math.min(120, delta)) * .004), event.clientX - rect.left, event.clientY - rect.top);
  }, { signal, passive: false });
  viewport.addEventListener('pointerdown', event => {
    if (event.button !== 0 || event.target.closest('button')) return;
    viewport.focus({ preventScroll: true });
    drag = { id: event.pointerId, x: event.clientX, y: event.clientY, ...offsetToStart() };
    viewport.setPointerCapture(event.pointerId); viewport.classList.add('dragging');
  }, { signal });
  function offsetToStart() { return { ox: offset.x, oy: offset.y }; }
  viewport.addEventListener('pointermove', event => {
    if (!drag || drag.id !== event.pointerId) return;
    offset = { x: drag.ox + event.clientX - drag.x, y: drag.oy + event.clientY - drag.y };
    autoFit = false; transform();
  }, { signal });
  function release() { drag = null; viewport.classList.remove('dragging'); }
  viewport.addEventListener('lostpointercapture', release, { signal });
  viewport.addEventListener('pointercancel', release, { signal });
  viewport.addEventListener('pointerup', release, { signal });
  viewport.addEventListener('keydown', event => {
    if (event.target !== viewport) return;
    const shifts = { ArrowLeft: [60, 0], ArrowRight: [-60, 0], ArrowUp: [0, 60], ArrowDown: [0, -60] };
    if (shifts[event.key]) {
      event.preventDefault(); offset.x += shifts[event.key][0]; offset.y += shifts[event.key][1]; autoFit = false; transform();
    } else if (event.key === '+' || event.key === '=') { event.preventDefault(); zoom(1.2); }
    else if (event.key === '-') { event.preventDefault(); zoom(1 / 1.2); }
    else if (event.key === '0') { event.preventDefault(); fit(); }
  }, { signal });
  window.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || (!root.contains(document.activeElement) && !root.classList.contains('fullscreen'))) return;
    if (root.classList.contains('fullscreen')) { expand.click(); expand.focus(); }
    else { selected = null; search.value = ''; results(); }
  }, { signal });
  const observer = new ResizeObserver(() => { if (autoFit) fit(); }); observer.observe(viewport);
  draw();
  return () => { controller.abort(); observer.disconnect(); root.replaceChildren(); root.classList.remove('fullscreen'); };
}
