import { turnIds } from './selection.js';
import { requestCost, PRICE_DATE, PRICE_SOURCE } from './pricing.js';
import { summarize, tokens, money, statusLabel, duration } from './summary.js';

export function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text != null) element.textContent = text;
  if (className) element.className = className;
  return element;
}

function row(label, value, className = '') {
  const element = node('div', null, `metric ${className}`);
  element.append(node('span', label), node('strong', value));
  return element;
}

export function renderSummary(requests, rates) {
  const stats = summarize(requests, rates);
  const box = node('section', null, 'usage-summary');
  const headline = node('div', null, 'usage-headline');
  headline.append(node('strong', stats.reported || !stats.requests ? tokens(stats.input + stats.output) : '—'), node('span', 'токенов', 'muted'));
  const cost = stats.priced || !stats.requests ? `≈ ${money(stats.cost)}` : '—';
  headline.append(node('strong', cost, 'cost-total'));
  box.append(headline);
  const metrics = node('div', null, 'usage-metrics');
  metrics.append(row('Вход', stats.reported ? tokens(stats.input) : '—'), row('Выход', stats.reported ? tokens(stats.output) : '—'));
  for (const [key, label] of [['cached', 'Из кэша · входит во вход'], ['write', 'Запись кэша · входит во вход'], ['reasoning', 'Рассуждения · входят в выход']]) {
    const partial = stats.detailsReported[key] > 0 && stats.detailsReported[key] < stats.requests;
    const metric = row(label, tokens(stats[key]), 'muted');
    if (partial) metric.append(node('small', `Данные ${stats.detailsReported[key]}/${stats.requests}`, 'detail-coverage'));
    metrics.append(metric);
  }
  box.append(metrics, row('Запросы', tokens(stats.requests)));
  if (stats.compactions) box.append(row('Из них сжатие контекста', tokens(stats.compactions), 'muted'));
  if (stats.errors) box.append(row('С ошибкой / отменой', tokens(stats.errors), 'muted'));
  if (stats.unfinished) box.append(row('Без записанного результата', tokens(stats.unfinished), 'muted'));
  if (stats.reported !== stats.requests) box.append(node('p', `Токены известны для ${stats.reported} из ${stats.requests} запросов. Остальной расход неизвестен.`, 'coverage muted'));
  if (stats.priced !== stats.requests) box.append(node('p', `Стоимость учтена для ${stats.priced} из ${stats.requests} запросов; показана известная часть.`, 'coverage muted'));
  return box;
}

export function renderModels(requests, rates) {
  const groups = new Map();
  for (const request of requests) {
    const key = `${request.model.provider} / ${request.model.model}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(request);
  }
  const details = node('details', null, 'usage-models');
  details.append(node('summary', 'По моделям'));
  for (const [name, items] of groups) {
    const stats = summarize(items, rates);
    const value = `${stats.reported ? tokens(stats.input + stats.output) : '—'} · ${stats.priced ? '≈ ' + money(stats.cost) : '—'}`;
    details.append(row(name, value));
  }
  return details;
}

function requestDetails(request, cost) {
  const details = node('div', null, 'request-metadata');
  details.append(row('Провайдер', request.model.provider), row('Тип', request.origin === 'compactor' ? 'Сжатие контекста' : 'Запрос модели'), row('Начало', new Date(request.started_at_ms).toLocaleString('ru-RU')), row('Длительность', duration(request)), row('Сообщений на входе', tokens(request.message_count)), row('Доступных инструментов', tokens(request.tool_count)), row('Уровень рассуждения', request.reasoning_effort ?? '—'), row('Лимит выхода', tokens(request.max_output_tokens)), row('Причина завершения', request.finish_reason ?? '—'));
  if (request.usage) {
    const usage = request.usage;
    const fields = [['Вход', 'input_tokens'], ['Из кэша', 'cached_input_tokens'], ['Запись кэша', 'cache_creation_input_tokens'], ['Выход', 'output_tokens'], ['Рассуждения', 'reasoning_output_tokens']];
    for (const [label, key] of fields) details.append(row(label, tokens(usage[key])));
  } else details.append(node('p', 'Провайдер не вернул данные о токенах. Нулевой расход не предполагается.', 'muted'));
  if (cost.value != null) {
    details.append(row('Стоимость входа', money(cost.input)), row('Стоимость выхода', money(cost.output)), row('Тариф', cost.custom ? 'Пользовательский' : `Standard API · ${PRICE_DATE}`));
    if (cost.long) details.append(node('p', 'Применён тариф длинного контекста.', 'muted'));
  } else details.append(node('p', cost.reason, 'muted'));
  details.append(row('ID запроса', request.exchange_id, 'request-id'), row('ID хода', request.turn_id ?? 'Вне хода', 'request-id'));
  return details;
}

export function renderRequests(requests, all, rates, wide, page, opened) {
  const box = node('div', null, 'usage-requests');
  const size = wide ? 20 : 5;
  const last = Math.max(0, Math.ceil(requests.length / size) - 1);
  const current = Math.min(page, last);
  const selected = (wide ? requests : [...requests].reverse()).slice(current * size, (current + 1) * size);
  const turns = turnIds(all);
  const numbers = new Map(all.map((item, index) => [item.exchange_id, index + 1]));
  if (!selected.length) box.append(node('p', all.length ? 'По этим фильтрам запросов нет.' : 'В этой сессии ещё нет записанных запросов.', 'muted'));
  for (const request of selected) {
    const cost = requestCost(request, rates), usage = request.usage;
    const item = node('details', null, 'usage-request');
    item.dataset.exchangeId = request.exchange_id;
    item.dataset.status = request.status;
    item.open = opened.has(request.exchange_id);
    const summary = node('summary');
    const number = numbers.get(request.exchange_id);
    const identity = node('span', null, 'request-identity');
    identity.append(node('strong', `#${number} · ${request.model.model}`));
    const state = node('span', null, 'request-state');
    const badge = node('span', statusLabel(request.status), 'request-status');
    badge.dataset.status = request.status;
    state.append(badge, node('span', duration(request), 'muted'));
    identity.append(state);
    if (wide) {
      const turn = request.turn_id == null ? 'Вне хода' : `Ход ${turns.indexOf(request.turn_id) + 1}`;
      const origin = request.origin === 'compactor' ? ' · Сжатие контекста' : '';
      identity.append(node('small', `${turn}${origin} · ${new Date(request.started_at_ms).toLocaleTimeString('ru-RU')}`, 'muted'));
    }
    summary.append(identity);
    if (wide) {
      for (const [label, value] of [['Вход', usage?.input_tokens], ['Кэш', usage?.cached_input_tokens], ['Запись', usage?.cache_creation_input_tokens], ['Выход', usage?.output_tokens], ['Мысли', usage?.reasoning_output_tokens]]) {
        const cell = node('span', null, 'request-cell'); cell.append(node('small', label), node('span', tokens(value))); summary.append(cell);
      }
    }
    const end = node('span', null, 'request-total');
    end.append(node('strong', usage ? tokens(usage.input_tokens + usage.output_tokens) : '—'), node('span', cost.value == null ? '—' : `≈ ${money(cost.value)}`, 'muted'));
    summary.append(end);
    item.append(summary, requestDetails(request, cost));
    box.append(item);
  }
  return { box, current, last };
}

export function priceNote() {
  const details = node('details', null, 'usage-price-note muted');
  details.append(node('summary', 'Оценка API, не списание'));
  const note = node('p');
  note.append(document.createTextNode('Оценка токенов по Standard API или своим тарифам; для подписки — условный эквивалент, не списание. Неразмеченный вход считается по обычной цене. Услуги встроенных tools, скидки и налоги не включены. '));
  const link = node('a', `Цены на ${PRICE_DATE}`); link.href = PRICE_SOURCE; link.target = '_blank'; link.rel = 'noreferrer';
  note.append(link);
  details.append(note);
  return details;
}
