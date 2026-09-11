import { requestCost } from './pricing.js';
import { durationMs } from './summary.js';

export const modelKey = request => JSON.stringify([request.model.provider, request.model.model]);
export const turnIds = requests => [...new Set(requests.map(item => item.turn_id).filter(Boolean))];

export function scopedRequests(snapshot, scope, model) {
  const turn = scope === 'latest' ? snapshot.latest_turn_id : scope;
  return snapshot.requests.filter(item =>
    (scope === 'all' || (turn != null && item.turn_id === turn)) &&
    (model === 'all' || modelKey(item) === model));
}

// The scope summary remains stable while diagnostic filters narrow the request list.
export function selectRequests(requests, { status = 'all', origin = 'all', query = '', sort = 'oldest' }, rates) {
  const terms = query.trim().toLocaleLowerCase('ru-RU').split(/\s+/).filter(Boolean);
  const selected = requests.filter(item => {
    const matchesStatus = status === 'all' ||
      (status === 'problems' ? item.status !== 'completed' :
        status === 'missing_usage' ? item.usage == null : item.status === status);
    const haystack = [item.exchange_id, item.turn_id, item.model.provider, item.model.model].join(' ').toLocaleLowerCase('ru-RU');
    return matchesStatus && (origin === 'all' || item.origin === origin) && terms.every(term => haystack.includes(term));
  });
  const value = item => {
    if (sort === 'duration') return durationMs(item);
    if (sort === 'tokens') return item.usage ? item.usage.input_tokens + item.usage.output_tokens : null;
    if (sort === 'cost') return requestCost(item, rates).value;
    return item.started_at_ms;
  };
  // Keep original journal order for ties. Unknown measurements always go last.
  return selected.map((request, index) => ({ request, index, value: value(request) }))
    .sort((a, b) => {
      if (a.value == null || b.value == null) return (a.value == null) - (b.value == null) || a.index - b.index;
      return (sort === 'oldest' ? a.value - b.value : b.value - a.value) || a.index - b.index;
    }).map(item => item.request);
}

export function requestExport(snapshot, requests, filters) {
  return JSON.stringify({ session_id: snapshot.session_id, revision: snapshot.revision, filters, requests }, null, 2);
}
