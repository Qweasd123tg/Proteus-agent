import { files, changes, patches } from './demo-data.js';

function listing(path) {
  const prefix = path ? `${path}/` : '', entries = new Map();
  for (const file of Object.keys(files)) {
    if (!file.startsWith(prefix)) continue;
    const [name, ...rest] = file.slice(prefix.length).split('/');
    entries.set(name, { name, path: prefix + name, kind: rest.length ? 'directory' : 'file' });
  }
  if (path && !entries.size) throw Error('Папка не найдена');
  const sorted = [...entries.values()].sort((a, b) => a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === 'directory' ? -1 : 1);
  return { path, entries: sorted, truncated: false };
}

export function createServices({ signal }) {
  return { 'agent.workspace.read': view => {
    const reply = async value => { signal.throwIfAborted(); view.throwIfAborted(); return structuredClone(value); };
    return {
      async list(path) { signal.throwIfAborted(); view.throwIfAborted(); return listing(path); },
      async read(path) {
        signal.throwIfAborted(); view.throwIfAborted();
        if (!Object.hasOwn(files, path)) throw Error('Файл не найден');
        return reply({ path, size: new TextEncoder().encode(files[path]).length, kind: 'text', text: files[path] });
      },
      changes: () => reply({ repository: true, truncated: false, entries: Object.entries(changes).map(([path, status]) => ({ path, status })) }),
      diff: path => reply({ path, kind: 'text', patch: patches[path] ?? '' }),
    };
  } };
}
