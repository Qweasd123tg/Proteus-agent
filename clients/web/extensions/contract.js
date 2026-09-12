export const API_VERSION = 1;

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
  object(value, ['apiVersion', 'id', 'name', 'description', 'entry', 'requires', 'settings', 'presentation'], 'Манифест');
  if (value.apiVersion !== API_VERSION) throw new Error(`Неподдерживаемая версия UI API: ${value.apiVersion}`);
  if (typeof value.id !== 'string' || !/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/.test(value.id)) throw new Error('Некорректный id расширения');
  if (typeof value.name !== 'string' || !value.name.trim()) throw new Error('Не указано название расширения');
  if (typeof value.description !== 'string') throw new Error('Не указано описание расширения');
  if (!Array.isArray(value.requires) || value.requires.some(item => typeof item !== 'string' || !item)) throw new Error('requires должен быть списком интерфейсов');
  if (new Set(value.requires).size !== value.requires.length) throw new Error('Повтор интерфейса в requires');
  if (value.presentation !== undefined && !['widget', 'panel'].includes(value.presentation)) throw new Error('Неизвестное представление расширения');
  let settings;
  if (value.settings !== undefined) {
    object(value.settings, ['entry', 'requires'], 'Настройки пакета');
    const required = value.settings.requires;
    if (!Array.isArray(required) || required.some(item => typeof item !== 'string' || !item) || new Set(required).size !== required.length) throw new Error('Некорректные интерфейсы настроек');
    settings = Object.freeze({ entry: resourceUrl(value.settings.entry, url), requires: Object.freeze([...required]) });
  }
  return Object.freeze({ ...value, ...(settings ? { settings } : {}), requires: Object.freeze([...value.requires]), entry: resourceUrl(value.entry, url) });
}

export function parseSettings(value, base) {
  object(value, ['apiVersion', 'panels'], 'Настройки расширений');
  if (value.apiVersion !== API_VERSION || !Array.isArray(value.panels)) throw new Error('Неподдерживаемый формат настроек расширений');
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

export function missingServices(manifest, services) {
  return manifest.requires.filter(name => !Object.hasOwn(services, name));
}
