const number = new Intl.NumberFormat('ru', { maximumFractionDigits: 1 });

export function duration(seconds, fallback) {
  if (seconds == null) return fallback;
  for (const [size, label] of [[86400, 'дн.'], [3600, 'ч'], [60, 'мин']]) {
    if (seconds % size === 0) return `${number.format(seconds / size)} ${label}`;
  }
  return `${number.format(seconds)} с`;
}

export function remaining(used) {
  return Math.max(0, Math.min(100, 100 - used));
}

export function percent(value) { return `${number.format(value)}%`; }

export function timestamp(seconds) {
  return new Date(seconds * 1000).toLocaleString('ru', {
    day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit',
  });
}

export function resetLabel(seconds, now = Date.now()) {
  if (seconds == null) return 'Время сброса не указано';
  if (seconds * 1000 <= now) return 'Время сброса прошло · ожидаем новые данные';
  return `Сброс ${timestamp(seconds)}`;
}
