// Device package storage is a client-host adapter, never an agent API.
export function nativeArchivePackages(invoke = globalThis.__TAURI__?.core?.invoke) {
  return {
    available: typeof invoke === 'function',
    async install(file, excludedIds, signal) {
      if (!invoke) throw Error('Установка ZIP доступна в настольном приложении Proteus.');
      if (!file || typeof file.arrayBuffer !== 'function' || !/\.zip$/i.test(file.name)) throw Error('Выберите ZIP-пакет расширения.');
      if (file.size > 64 * 1024 * 1024) throw Error('ZIP превышает 64 МиБ.');
      signal.throwIfAborted();
      const bytes = new Uint8Array(await file.arrayBuffer());
      signal.throwIfAborted();
      const chunks = [];
      for (let index = 0; index < bytes.length; index += 32768) chunks.push(String.fromCharCode(...bytes.subarray(index, index + 32768)));
      const installed = await invoke('install_ui_extension', { archive: btoa(chunks.join('')), excludedIds });
      if (signal.aborted) {
        await invoke('remove_ui_extension', { key: installed.key });
        signal.throwIfAborted();
      }
      return installed;
    },
    async remove(key) {
      if (!invoke) throw Error('Хранилище ZIP-пакетов недоступно.');
      await invoke('remove_ui_extension', { key });
    },
  };
}
