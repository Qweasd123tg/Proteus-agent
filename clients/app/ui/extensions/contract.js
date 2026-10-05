export const API_VERSION = 2;
export const SETTINGS_VERSION = 1;
export const SURFACES = ['compact', 'workspace', 'settings', 'composer-model', 'composer-access'];

function object(value, fields, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label}: ожидается объект`);
  for (const key of Object.keys(value)) {
    if (!fields.includes(key)) throw new Error(`${label}: неизвестное поле ${key}`);
  }
}

export function resourceUrl(value, base) {
  if (typeof value !== 'string' || !value.trim()) throw new Error('Не указан URL');
  const url = new URL(value, base);
  const baseUrl = new URL(base);
  if (!['http:', 'https:'].includes(url.protocol) && url.protocol !== baseUrl.protocol) {
    throw new Error('Нужен HTTP(S) URL или ресурс текущего клиента');
  }
  if (['data:', 'javascript:', 'file:', 'blob:'].includes(url.protocol) || url.username || url.password || url.hash) {
    throw new Error('Недопустимый URL расширения');
  }
  return url.href;
}

export function parseManifest(value, url) {
  object(value, ['apiVersion', 'id', 'name', 'description', 'icon', 'views'], 'Манифест');
  if (value.apiVersion !== API_VERSION) throw new Error(`Неподдерживаемая версия UI API: ${value.apiVersion}`);
  if (typeof value.id !== 'string' || !/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/.test(value.id)) throw new Error('Некорректный id расширения');
  if (typeof value.name !== 'string' || !value.name.trim()) throw new Error('Не указано название расширения');
  if (typeof value.description !== 'string') throw new Error('Не указано описание расширения');
  if (value.icon !== undefined && (typeof value.icon !== 'string' || !/^[a-z][a-z0-9-]*$/.test(value.icon))) throw new Error('Некорректный значок расширения');
  if (!Array.isArray(value.views) || !value.views.length) throw new Error('Нужен список представлений расширения');
  const declared = new Set();
  const views = value.views.map(view => {
    object(view, ['surfaces', 'entry', 'requires', 'layout', 'isolation'], 'Представление');
    if (!Array.isArray(view.surfaces) || !view.surfaces.length || view.surfaces.some(surface => !SURFACES.includes(surface) || declared.has(surface)) || new Set(view.surfaces).size !== view.surfaces.length) throw new Error('Неизвестная или повторная поверхность');
    if (view.surfaces.length > 1 && view.surfaces.some(surface => !['compact', 'workspace'].includes(surface))) throw new Error('Общий экземпляр допустим только для compact и workspace');
    for (const surface of view.surfaces) declared.add(surface);
    if (!Array.isArray(view.requires) || view.requires.some(name => typeof name !== 'string' || !name) || new Set(view.requires).size !== view.requires.length) throw new Error('Некорректные интерфейсы представления');
    if (!['scroll', 'fill', 'form', 'editor'].includes(view.layout)) throw new Error('Неизвестный layout представления');
    if (!['shadow', 'light'].includes(view.isolation)) throw new Error('Неизвестная изоляция представления');
    return Object.freeze({ ...view, entry: resourceUrl(view.entry, url), surfaces: Object.freeze([...view.surfaces]), requires: Object.freeze([...view.requires]) });
  });
  return Object.freeze({ ...value, views: Object.freeze(views) });
}

export function parseSettings(value, base) {
  object(value, ['apiVersion', 'panels'], 'Настройки расширений');
  if (value.apiVersion !== SETTINGS_VERSION || !Array.isArray(value.panels)) throw new Error('Неподдерживаемый формат настроек расширений');
  const ids = new Set();
  return value.panels.map(panel => {
    object(panel, ['id', 'url', 'enabled', 'collapsed', 'location'], 'Панель');
    if (typeof panel.id !== 'string' || !panel.id || ids.has(panel.id)) throw new Error('Пустой или повторяющийся id панели');
    if (typeof panel.enabled !== 'boolean' || typeof panel.collapsed !== 'boolean') throw new Error('Некорректное состояние панели');
    if (panel.location !== undefined && !['left', 'right'].includes(panel.location)) throw new Error('Некорректная область панели');
    ids.add(panel.id);
    return { ...panel, location: panel.location ?? 'right', url: resourceUrl(panel.url, base) };
  });
}

export function missingServices(view, services) {
  return view.requires.filter(name => !Object.hasOwn(services, name));
}

export function viewForSurface(manifest, surface) {
  return manifest?.views?.find(view => view.surfaces.includes(surface));
}
export function hasSurface(manifest, surface) {
  return !!viewForSurface(manifest, surface);
}
