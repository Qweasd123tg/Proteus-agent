import { requestCost } from './pricing.js';

export function summarize(requests, rates) {
  const result = { requests: requests.length, reported: 0, input: 0, output: 0, cached: null, write: null, reasoning: null, cost: 0, priced: 0, unfinished: 0, errors: 0, compactions: 0 };
  result.detailsReported = { cached: 0, write: 0, reasoning: 0 };
  for (const request of requests) {
    if (request.status === 'unfinished') result.unfinished++;
    else if (request.status !== 'completed') result.errors++;
    if (request.origin === 'compactor') result.compactions++;
    if (request.usage) {
      result.reported++;
      result.input += request.usage.input_tokens;
      result.output += request.usage.output_tokens;
      for (const [key, source] of [['cached', 'cached_input_tokens'], ['write', 'cache_creation_input_tokens'], ['reasoning', 'reasoning_output_tokens']]) {
        if (request.usage[source] != null) { result[key] = (result[key] ?? 0) + request.usage[source]; result.detailsReported[key]++; }
      }
    }
    const cost = requestCost(request, rates);
    if (cost.value != null) { result.cost += cost.value; result.priced++; }
  }
  return result;
}

export const tokens = value => value == null ? '—' : new Intl.NumberFormat('ru-RU').format(value);
export const money = value => value == null ? '—' : new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 6 }).format(value);
export const statusLabel = status => ({ completed: 'Готово', error: 'Ошибка', canceled: 'Отменён', timeout: 'Таймаут', unfinished: 'Нет результата' })[status] ?? status;
export function durationMs(request) {
  if (request.finished_at_ms == null) return null;
  return Math.max(0, request.finished_at_ms - request.started_at_ms);
}
export function duration(request) {
  const value = durationMs(request);
  return value == null ? '—' : `${new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 1 }).format(value / 1000)} с`;
}
