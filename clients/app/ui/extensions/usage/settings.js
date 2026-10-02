import { BUILTIN_RATES, PRICE_DATE, PRICE_SOURCE, readRates, validateRates } from './pricing.js';
import { node } from './render.js';

export function mount({ root, storage, signal }) {
  root.append(node('style', `
    .rates-form { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
    label { display: grid; gap: 5px; color: var(--text-muted); font-size: 12px; min-width: 0; }
    input, select { box-sizing: border-box; width: 100%; min-width: 0; border: 1px solid var(--border-subtle); border-radius: 8px; background: var(--bg-panel-soft); color: var(--text-main); padding: 8px; font: inherit; }
    .rates-list { margin: 16px 0; } .rate-row { display: flex; align-items: center; gap: 8px; padding: 8px 0; border-bottom: 1px solid var(--border-subtle); }
    .rate-row span { flex: 1; overflow-wrap: anywhere; min-width: 0; }
    .rate-actions { display: flex; gap: 8px; grid-column: 1 / -1; }
    .rate-source { margin: 12px 0; font-size: 12px; } p { line-height: 1.5; }
    @media (max-width: 520px) { .rates-form { grid-template-columns: 1fr; } }
  `));
  root.append(node('p', 'Стоимость — оценка токенов в USD. Тарифы ниже применяются ко всему отчёту, включая прошлые запросы. Для подписки это условная стоимость по API, не счёт к оплате.', 'muted'));
  const source = node('p', null, 'rate-source');
  const link = node('a', `Standard API OpenAI · ${PRICE_DATE}`); link.href = PRICE_SOURCE; link.target = '_blank'; link.rel = 'noreferrer'; source.append(link);
  root.append(source);
  const presetLabel = node('label', 'Взять за основу');
  const preset = node('select');
  const blank = node('option', 'Свой тариф'); blank.value = ''; preset.append(blank);
  for (const rate of BUILTIN_RATES) { const option = node('option', rate.model); option.value = rate.model; preset.append(option); }
  presetLabel.append(preset); root.append(presetLabel);
  const list = node('div', null, 'rates-list');
  const form = node('form', null, 'rates-form');
  const inputs = {};
  const fields = [
    ['provider', 'Провайдер · пусто = любой', 'text', ''], ['model', 'Модель · точное имя', 'text', ''],
    ['input', 'Вход · $ за 1 млн', 'number', 0], ['output', 'Выход · $ за 1 млн', 'number', 0],
    ['cached', 'Чтение кэша · $ за 1 млн', 'number', 0], ['write', 'Запись кэша · $ за 1 млн', 'number', 0],
    ['threshold', 'Порог длинного контекста · 0 = нет', 'number', 0], ['input_multiplier', 'Множитель входа и кэша выше порога', 'number', 1],
    ['output_multiplier', 'Множитель выхода выше порога', 'number', 1],
  ];
  for (const [key, title, type, value] of fields) {
    const label = node('label', title), input = node('input');
    input.type = type; input.value = String(value); input.name = key;
    input.setAttribute('aria-label', title);
    if (type === 'number') { input.min = '0'; input.step = key === 'threshold' ? '1' : 'any'; }
    input.required = key !== 'provider'; inputs[key] = input; label.append(input); form.append(label);
  }
  const actions = node('div', null, 'rate-actions');
  const save = node('button', 'Сохранить тариф'); save.type = 'submit';
  const clear = node('button', 'Очистить форму'); clear.type = 'button'; actions.append(save, clear); form.append(actions);
  const status = node('p', '', 'muted'); status.setAttribute('role', 'status');
  root.append(list, form, status);
  const reset = node('button', 'Сбросить свои тарифы'); reset.type = 'button';
  reset.addEventListener('click', () => {
    try { storage.remove('rates'); status.className = 'muted'; status.textContent = 'Свои тарифы удалены.'; display(); }
    catch (error) { status.className = 'error'; status.textContent = error.message; }
  }, { signal });
  root.append(reset);
  let rowController;
  function fill(rate) { for (const [key, , , value] of fields) inputs[key].value = String(rate?.[key] ?? value); }
  function display() {
    rowController?.abort(); rowController = new AbortController();
    list.replaceChildren();
    let rates;
    try { rates = readRates(storage); }
    catch (error) { status.className = 'error'; status.textContent = `Не удалось прочитать тарифы: ${error.message}`; return; }
    if (!rates.length) list.append(node('p', 'Своих тарифов нет. Для известных моделей используется опубликованный Standard API.', 'muted'));
    rates.forEach((rate, index) => {
      const row = node('div', null, 'rate-row');
      row.append(node('span', `${rate.provider ? rate.provider + ' / ' : ''}${rate.model} · вход $${rate.input} / выход $${rate.output}`));
      const edit = node('button', 'Изменить'), remove = node('button', 'Удалить'); edit.type = remove.type = 'button';
      edit.addEventListener('click', () => fill(rate), { signal: rowController.signal });
      remove.addEventListener('click', () => {
        try { storage.set('rates', JSON.stringify(rates.filter((_, i) => i !== index))); status.textContent = 'Свой тариф удалён.'; display(); }
        catch (error) { status.className = 'error'; status.textContent = error.message; }
      }, { signal: rowController.signal });
      row.append(edit, remove); list.append(row);
    });
  }
  preset.addEventListener('change', () => fill(BUILTIN_RATES.find(rate => rate.model === preset.value)), { signal });
  clear.addEventListener('click', () => { fill(); preset.value = ''; }, { signal });
  form.addEventListener('submit', event => {
    event.preventDefault();
    try {
      const next = Object.fromEntries(fields.map(([key, , type]) => [key, type === 'number' ? Number(inputs[key].value) : inputs[key].value.trim()]));
      const rates = readRates(storage).filter(rate => rate.model !== next.model || rate.provider !== next.provider);
      storage.set('rates', JSON.stringify(validateRates([...rates, next])));
      status.className = 'muted'; status.textContent = 'Тариф сохранён. Стоимость в отчёте пересчитана.'; display();
    } catch (error) { status.className = 'error'; status.textContent = error.message; }
  }, { signal });
  display();
  return () => rowController?.abort();
}
