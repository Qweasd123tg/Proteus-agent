// Presentation of the public topology snapshot. Edges are never inferred here.
export const NODE_WIDTH = 180;
export const NODE_HEIGHT = 64;
const GAP_X = 224;
const GAP_Y = 80;

export function graphModel(snapshot) {
  const nodes = new Map();
  function add(id, kind, label, subtitle, description, facts = [], extra = {}) {
    nodes.set(id, { id, kind, label, subtitle, description, facts, ...extra });
  }
  add('config', 'config', snapshot.profile || 'Профиль', 'Конфигурация', 'Текущая сборка агента', [
    ['Каталог', snapshot.cwd], ['Файл', snapshot.config_path || 'Автоматический выбор'],
    ['Ревизия', snapshot.module_epoch], ['Доступ', snapshot.permission_mode],
    ...snapshot.config_files.map(file => ['Источник', file]),
  ]);
  add('tools', 'tools', 'Инструменты', `${snapshot.tools.filter(tool => tool.registered).length} зарегистрировано`, 'Инструменты текущей сборки');
  for (const slot of [...snapshot.slots].sort((a, b) => a.order - b.order || a.id.localeCompare(b.id))) {
    add(`slot:${slot.id}`, 'slot', slot.title || slot.id, slot.active_module || 'Модуль не выбран', slot.responsibility, [
      ['Slot', slot.id], ['Категория', slot.category], ['Обязательный', slot.required ? 'Да' : 'Нет'],
      ['Выбранный модуль', slot.active_module || 'Не выбран'],
    ], { category: slot.category, active: !!slot.active_module, missing: slot.required && !slot.active_module });
  }
  for (const module of snapshot.modules) {
    add(`module:${module.slot}:${module.id}`, 'module', module.id, `${module.slot} · ${module.active ? 'выбран' : 'доступен'}`, module.description || '', [
      ['Slot', module.slot], ['Источник', module.source.kind], ['Версия', module.version], ['API', module.api_version],
      ['Возможности', module.capabilities.join(', ') || 'Не указаны'],
    ], { slot: module.slot, active: module.active });
  }
  for (const tool of snapshot.tools) {
    add(`tool:${tool.name}`, 'tool', tool.name, tool.registered ? (tool.enabled ? 'Включён' : 'Отключён') : 'Не зарегистрирован', tool.description, [
      ['Источник', tool.source], ['Safety', tool.safety], ['Регистрация', tool.registered ? 'Да' : 'Нет'],
      ['Включён', tool.enabled ? 'Да' : 'Нет'],
    ], { active: tool.registered && tool.enabled, schema: tool.input_schema });
  }
  // Retain server-owned endpoints even when a newer server adds a node type.
  for (const edge of snapshot.edges) {
    for (const id of [edge.from, edge.to]) {
      if (!nodes.has(id)) add(id, 'other', id, 'Связь из snapshot', 'Узел без подробного описания в текущем snapshot.');
    }
  }
  return { nodes, edges: snapshot.edges };
}

export function layoutGraph(model, scope, includeInactive = false) {
  const nodes = [];
  function place(node, col, row) { nodes.push({ ...node, x: 32 + col * GAP_X, y: 56 + row * GAP_Y }); }
  const all = [...model.nodes.values()];
  if (scope === 'modules') {
    let row = 0;
    const slots = new Set([...all.filter(node => node.kind === 'slot').map(node => node.id.slice(5)), ...all.filter(node => node.kind === 'module').map(node => node.slot)]);
    for (const slotId of slots) {
      const slot = model.nodes.get(`slot:${slotId}`);
      const modules = all.filter(node => node.kind === 'module' && node.slot === slotId && (includeInactive || node.active));
      if (!modules.length && !includeInactive) continue;
      if (slot) place(slot, 0, row);
      modules.forEach((node, index) => place(node, 1 + index % 3, row + Math.floor(index / 3)));
      row += Math.max(1, Math.ceil(modules.length / 3)) + .35;
    }
  } else if (scope === 'tools') {
    const tools = all.filter(node => node.kind === 'tool' && (includeInactive || node.active));
    place(model.nodes.get('tools'), 0, 0);
    tools.forEach((node, index) => place(node, 1 + index % 3, Math.floor(index / 3)));
  } else {
    const columns = [[], [], [], []];
    for (const node of all.filter(node => !['module', 'tool'].includes(node.kind))) {
      const column = node.kind === 'config' || node.category === 'orchestrator' ? 0
        : node.kind === 'tools' ? 2 : ['backend', 'post_turn'].includes(node.category) ? 3 : 1;
      columns[column].push(node);
    }
    const maxRows = Math.max(...columns.map(column => column.length));
    columns.forEach((column, col) => column.forEach((node, row) => place(node, col, row + (maxRows - column.length) / 2)));
  }
  const visible = new Map(nodes.map(node => [node.id, node]));
  return {
    nodes,
    edges: model.edges.filter(edge => visible.has(edge.from) && visible.has(edge.to)),
    width: Math.max(600, ...nodes.map(node => node.x + NODE_WIDTH + 32)),
    height: Math.max(300, ...nodes.map(node => node.y + NODE_HEIGHT + 32)),
  };
}

export function relatedNodes(model, selected) {
  const result = new Set([selected]);
  for (const edge of model.edges) {
    if (edge.from === selected) result.add(edge.to);
    if (edge.to === selected) result.add(edge.from);
  }
  return result;
}

export function matches(node, query) {
  return `${node.label} ${node.subtitle} ${node.id} ${node.description}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
}

export function edgeLabel(edge) {
  return edge.label || ({ selects: 'Выбор', active_module: 'Выбран', available_module: 'Доступен',
    runtime: 'Используется', registered_tool: 'Зарегистрирован', unregistered_tool: 'Не зарегистрирован',
    enables: 'Включает', uses: 'Использует' })[edge.kind] || edge.kind;
}
