import { edgeLabel } from './model.js';

export function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text != null) node.textContent = text;
  if (className) node.className = className;
  return node;
}

export function renderDetails(root, model, selected, select) {
  root.replaceChildren();
  const node = model.nodes.get(selected);
  if (!node) {
    root.append(element('span', 'ОБЪЕКТ', 'graph-eyebrow'), element('h3', 'Исследуйте сборку'),
      element('p', 'Выберите узел на карте: здесь появятся его свойства и связанные объекты.', 'graph-muted'),
      element('p', 'Ищите по имени слота, модуля или инструмента. Связи показывают устройство сборки, а не ход конкретной сессии.', 'graph-muted'));
    return;
  }
  root.append(element('span', ({ slot: 'СЛОТ', module: 'МОДУЛЬ', tool: 'ИНСТРУМЕНТ', config: 'ПРОФИЛЬ', tools: 'ИНСТРУМЕНТЫ' })[node.kind] || 'ОБЪЕКТ', 'graph-eyebrow'),
    element('h3', node.label), element('p', node.description, 'graph-muted'));
  const facts = element('dl', null, 'graph-facts');
  for (const [label, value] of node.facts) facts.append(element('dt', label), element('dd', String(value ?? '—')));
  root.append(facts);
  const edges = model.edges.filter(edge => edge.from === selected || edge.to === selected);
  root.append(element('h4', `Связи · ${edges.length}`));
  for (const edge of edges) {
    const outgoing = edge.from === selected;
    const other = model.nodes.get(outgoing ? edge.to : edge.from);
    const button = element('button', null, 'graph-relation');
    button.type = 'button';
    button.dataset.relatedId = other.id;
    button.append(element('span', `${outgoing ? '→' : '←'} ${other.label}`), element('small', edgeLabel(edge)));
    button.addEventListener('click', () => select(other.id));
    root.append(button);
  }
  if (node.schema) {
    const details = element('details', null, 'graph-schema');
    details.append(element('summary', 'Параметры инструмента'), element('pre', JSON.stringify(node.schema, null, 2)));
    root.append(details);
  }
}
