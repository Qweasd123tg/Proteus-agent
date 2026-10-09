// The package owns fixtures and service implementations; the host only loads them.
export async function loadPreviewServices(preview, signal, load = url => import(url)) {
  signal.throwIfAborted();
  const implementation = await load(preview.entry);
  signal.throwIfAborted();
  if (typeof implementation.createServices !== 'function') throw Error('Превью не экспортирует createServices');
  const services = await implementation.createServices(Object.freeze({ signal }));
  signal.throwIfAborted();
  if (!services || typeof services !== 'object' || Array.isArray(services) || Object.values(services).some(factory => typeof factory !== 'function')) {
    throw Error('createServices должен вернуть объект фабрик демо-сервисов');
  }
  return Object.freeze({ ...services });
}
