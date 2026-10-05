// Configuration values stay sparse: displaying a default never writes it.
export const sameValue = (a, b) => JSON.stringify(a) === JSON.stringify(b);
export function readPath(object, path) {
  for (const key of path) {
    if (object === null || typeof object !== "object" || !Object.hasOwn(object, key)) return undefined;
    object = object[key];
  }
  return object;
}
export function writePath(object, path, value) {
  const key = path[0];
  if (path.length === 1) {
    if (value === undefined) delete object[key];
    else Object.defineProperty(object, key, { value, writable: true, enumerable: true, configurable: true });
  } else {
    if (!Object.hasOwn(object, key) || object[key] === null || typeof object[key] !== "object" || Array.isArray(object[key])) {
      if (value === undefined) return;
      Object.defineProperty(object, key, { value: {}, writable: true, enumerable: true, configurable: true });
    }
    writePath(object[key], path.slice(1), value);
    if (!Object.keys(object[key]).length) delete object[key];
  }
}
export function parseScalar(schema, text) {
  if (["string", "boolean"].includes(schema.type)) return { value: text };
  if (schema.type === "integer" || schema.type === "number") {
    const number = Number(text);
    if (!text.trim() || !Number.isFinite(number)) return { error: "Введите число" };
    if (schema.type === "integer" && !Number.isSafeInteger(number)) return { error: "Введите целое число в точном диапазоне JavaScript" };
    if (schema.minimum != null && number < schema.minimum) return { error: `Минимум: ${schema.minimum}` };
    if (schema.maximum != null && number > schema.maximum) return { error: `Максимум: ${schema.maximum}` };
    return { value: number };
  }
  try { return { value: JSON.parse(text) }; }
  catch { return { error: "Введите корректное JSON-значение" }; }
}
export function valueError(schema, value) {
  switch (schema.type) {
    case "boolean": return typeof value === "boolean" ? null : "Ожидается да или нет";
    case "string": return typeof value === "string" ? null : "Ожидается текст";
    case "integer": case "number":
      return typeof value === "number" ? parseScalar(schema, String(value)).error : "Ожидается число";
    case "enum": return schema.options.some(option => sameValue(option.value, value)) ? null : "Выберите допустимое значение";
    case "array": return Array.isArray(value) ? value.map(item => valueError(schema.items, item)).find(Boolean) : "Ожидается список";
    case "object":
      if (!value || typeof value !== "object" || Array.isArray(value)) return "Ожидается объект";
      return schema.fields.map(field => {
        const next = Object.hasOwn(value, field.key) ? value[field.key] : field.default;
        return next == null && !Object.hasOwn(value, field.key) ? (field.required ? `Задайте ${field.title}` : null) : valueError(field.value, next);
      }).find(Boolean);
    default: return null;
  }
}
export function initialValue(schema) {
  switch (schema.type) {
    case "boolean": return false;
    case "integer": case "number": return schema.minimum ?? 0;
    case "enum": return schema.options[0]?.value;
    case "array": return [];
    case "object": return {};
    case "json": return {};
    default: return "";
  }
}
