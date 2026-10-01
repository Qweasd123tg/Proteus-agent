export function object(value, required, optional = [], label = "object") {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const allowed = new Set([...required, ...optional]);
  if (required.some((key) => !Object.hasOwn(value, key)) ||
      Object.keys(value).some((key) => !allowed.has(key))) {
    throw new Error(`${label} has missing or unsupported fields`);
  }
  return value;
}

export function nonblank(value, label) {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${label} must be nonblank`);
  return value;
}

export function frozenCopy(value) {
  const copy = structuredClone(value);
  const freeze = (item) => {
    if (item && typeof item === "object") {
      Object.values(item).forEach(freeze);
      Object.freeze(item);
    }
  };
  freeze(copy);
  return copy;
}

export function hostId(id, generation, allowZero = false) {
  const match = /^h:(0|[1-9]\d*):(0|[1-9]\d*)$/.exec(id);
  if (!match || match[1] !== generation || (!allowZero && match[2] === "0")) {
    throw new Error("invalid host invocation id or stale generation");
  }
  return match;
}
