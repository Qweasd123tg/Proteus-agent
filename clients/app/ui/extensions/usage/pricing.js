// Standard text-token API equivalents, verified 2026-09-10. Not a billing API.
export const PRICE_SOURCE = 'https://developers.openai.com/api/docs/pricing';
export const PRICE_DATE = '2026-09-10';
export const BUILTIN_RATES = [
  ['gpt-6-astra', 10, 1, 12.5, 50],
  ['gpt-5.6-sol', 4, .4, 5, 20],
  ['gpt-5.6', 4, .4, 5, 20],
  ['gpt-5.6-terra', 2, .2, 2.5, 12],
  ['gpt-5.6-luna', .2, .02, .25, 1.2],
].map(([model, input, cached, write, output]) => Object.freeze({
  provider: '', model, input, cached, write, output,
  threshold: 272000, input_multiplier: 2, output_multiplier: 1.5,
}));

export function validateRates(value) {
  if (!Array.isArray(value)) throw new Error('Тарифы должны быть списком');
  const seen = new Set();
  return value.map(rate => {
    const fields = ['provider', 'model', 'input', 'cached', 'write', 'output', 'threshold', 'input_multiplier', 'output_multiplier'];
    if (!rate || typeof rate !== 'object' || Object.keys(rate).some(key => !fields.includes(key))) throw new Error('Неизвестная форма тарифа');
    if (typeof rate.provider !== 'string' || typeof rate.model !== 'string' || !rate.model.trim()) throw new Error('Укажите модель');
    for (const key of fields.slice(2)) {
      if (!Number.isFinite(rate[key]) || rate[key] < 0) throw new Error('Цены и множители должны быть неотрицательными числами');
    }
    if (!Number.isSafeInteger(rate.threshold)) throw new Error('Порог должен быть целым числом токенов');
    const clean = { ...rate, provider: rate.provider.trim(), model: rate.model.trim() };
    const key = JSON.stringify([clean.provider, clean.model]);
    if (seen.has(key)) throw new Error('Повтор тарифа для одной модели и провайдера');
    seen.add(key);
    return clean;
  });
}

export function readRates(storage) {
  const text = storage.get('rates');
  return text === null ? [] : validateRates(JSON.parse(text));
}

export function requestCost(request, rates) {
  const usage = request.usage;
  if (!usage) return { value: null, reason: 'Нет данных о токенах' };
  const matches = rate => rate.model === request.model.model;
  const custom = rates.find(rate => matches(rate) && rate.provider === request.model.provider)
    ?? rates.find(rate => matches(rate) && rate.provider === '');
  const rate = custom ?? BUILTIN_RATES.find(matches);
  if (!rate) return { value: null, reason: 'Тариф не задан' };
  const input = usage.input_tokens, output = usage.output_tokens;
  const cached = usage.cached_input_tokens ?? 0, write = usage.cache_creation_input_tokens ?? 0;
  if (![input, output, cached, write].every(value => Number.isSafeInteger(value) && value >= 0) || cached + write > input) {
    return { value: null, reason: 'Некорректная разбивка токенов' };
  }
  const long = rate.threshold > 0 && input > rate.threshold;
  const inputCost = ((input - cached - write) * rate.input + cached * rate.cached + write * rate.write) * (long ? rate.input_multiplier : 1) / 1e6;
  const outputCost = output * rate.output * (long ? rate.output_multiplier : 1) / 1e6;
  if (!Number.isFinite(inputCost + outputCost)) return { value: null, reason: 'Слишком большое значение тарифа' };
  return { value: inputCost + outputCost, input: inputCost, output: outputCost, long, custom: !!custom, rate };
}
