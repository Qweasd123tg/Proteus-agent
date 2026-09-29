// Physical key codes make shortcuts independent of the current keyboard layout.
export const commands = [
  {id:'new-chat', label:'Новый чат', group:'Чат', binding:'Mod+Shift+KeyN'},
  {id:'focus-composer', label:'Фокус в поле сообщения', group:'Чат', binding:'Mod+KeyL'},
  {id:'stop', label:'Остановить ответ', group:'Чат', binding:'Escape'},
  {id:'sidebar', label:'Показать список чатов', group:'Навигация', binding:'Mod+KeyB'},
  {id:'workspace', label:'Показать боковую область', group:'Навигация', binding:'Mod+Shift+KeyB'},
  {id:'settings', label:'Открыть настройки', group:'Навигация', binding:'Mod+Comma'},
  {id:'project', label:'Открыть проект', group:'Приложение', binding:'Mod+Shift+KeyO', native:true},
  {id:'inspector', label:'Открыть Inspector', group:'Приложение', binding:'Mod+Shift+KeyI'},
  {id:'quit', label:'Выйти из Proteus', group:'Приложение', binding:'Mod+KeyQ', native:true},
];
export const defaults = () => Object.fromEntries(commands.map(c => [c.id, c.binding]));
export function fromEvent(event, mac = false) {
  if (event.isComposing || event.repeat || event.getModifierState?.('AltGraph')) return null;
  const code = event.code || (['Escape','Enter','Tab'].includes(event.key) ? event.key : '');
  if (!/^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-2])|Escape|Enter|Tab|Comma|Period|Slash|Backquote|BracketLeft|BracketRight|Backslash|Minus|Equal|Space|Arrow(Up|Down|Left|Right))$/.test(code)) return null;
  const parts = [];
  if (mac ? event.metaKey : event.ctrlKey) parts.push('Mod');
  if (mac ? event.ctrlKey : event.metaKey) parts.push(mac ? 'Ctrl' : 'Meta');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  return [...parts, code].join('+');
}
export function label(binding, mac = false) {
  if (!binding) return 'Не назначено';
  return binding.split('+').map(key => ({Mod:mac?'Cmd':'Ctrl',Escape:'Esc',Comma:',',Period:'.',Slash:'/',Backquote:'`',BracketLeft:'[',BracketRight:']',Backslash:'\\',Minus:'−',Equal:'=',Space:'Пробел',ArrowUp:'↑',ArrowDown:'↓',ArrowLeft:'←',ArrowRight:'→'}[key] || key.replace(/^(Key|Digit)/,''))).join(' + ');
}
export function bindingError(binding) {
  if (binding === null || binding === 'Escape') return '';
  if (typeof binding !== 'string' || !/^(Mod\+)?(Ctrl\+|Meta\+)?(Alt\+)?(Shift\+)?(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-2])|Comma|Period|Slash|Backquote|BracketLeft|BracketRight|Backslash|Minus|Equal|Space|Arrow(Up|Down|Left|Right))$/.test(binding)) return 'Это сочетание недоступно. Enter и Tab сохраняют своё назначение.';
  if (!/(Mod|Ctrl|Meta|Alt)\+/.test(binding) && !/^F\d+$/.test(binding)) return 'Используйте Ctrl/Cmd, Alt или функциональную клавишу.';
  if (/^(Mod|Ctrl|Meta)\+(Key[ACVXYZ]|KeyR|KeyW|KeyT|KeyN|KeyF|KeyP|Digit[0-9]|Minus|Equal)$/.test(binding) || binding === 'Alt+F4' || binding === 'Mod+Shift+KeyT') return 'Сочетание зарезервировано для редактирования, браузера или окна.';
  return '';
}
export function validate(bindings) {
  if (!bindings || typeof bindings !== 'object' || Array.isArray(bindings) || Object.keys(bindings).length !== commands.length) throw Error('Неверный формат сочетаний клавиш. Сбросьте настройки сочетаний.');
  const seen = new Set();
  for (const {id} of commands) {
    if (!(id in bindings)) throw Error('Неизвестный набор команд. Сбросьте настройки сочетаний.');
    const value = bindings[id], error = bindingError(value);
    if (error) throw Error(error);
    if (value && seen.has(value)) throw Error('Несколько команд используют одно сочетание.');
    if (value) seen.add(value);
  }
  return bindings;
}
export function rebind(bindings, id, binding) {
  const conflict = commands.find(c => c.id !== id && binding && bindings[c.id] === binding);
  if (conflict) throw Error(`Уже назначено: «${conflict.label}». Сначала измените или отключите его сочетание.`);
  return validate({...bindings, [id]:binding});
}
