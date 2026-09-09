import { mountExtensions } from './host.js';

// Адаптер этой витрины. Credentials остаются в transport-коде клиента;
// расширение получает только объявленный интерфейс чтения /config.
export function mountWebExtensions(root, readConfig) {
  const services = {
    'agent.config.read': signal => Object.freeze({
      async read() {
        signal.throwIfAborted();
        const config = await readConfig(signal);
        signal.throwIfAborted();
        return JSON.parse(config);
      },
    }),
  };
  return mountExtensions(root, services);
}
