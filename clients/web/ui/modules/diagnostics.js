const titles = {
  usage: "Расход и контекст",
  analysis: "Анализ ходов",
  configs: "Сборка агента",
  architecture: "Архитектура",
};
export function diagnosticsService(readUrl, signal) {
  return Object.freeze({
    mount(view, root) {
      signal.throwIfAborted();
      if (!Object.hasOwn(titles, view))
        throw Error("Неизвестный раздел диагностики");
      if (!readUrl()) throw Error("Диагностика ещё не подключена");
      const url = new URL(readUrl(), location.href);
      url.searchParams.set("embedded", "true");
      url.searchParams.set("view", view);
      url.searchParams.set("chat", location.origin);
      const frame = document.createElement("iframe");
      frame.className = "diagnostic-frame";
      frame.title = titles[view];
      frame.src = url.href;
      root.append(frame);
      const listener = (event) => {
        if (
          event.source !== frame.contentWindow ||
          event.origin !== url.origin ||
          event.data?.type !== "proteus-open-chat"
        )
          return;
        if (typeof event.data.href !== "string") return;
        const target = new URL(event.data.href, url);
        if (
          target.protocol === "proteus-desktop:" &&
          target.pathname === "chat"
        ) {
          const chat = new URL("/index.html", location.href),
            session = target.searchParams.get("session_dir");
          if (session) chat.searchParams.set("session_dir", session);
          location.href = chat.href;
        } else if (["http:", "https:"].includes(target.protocol))
          window.location.href = target.href;
      };
      window.addEventListener("message", listener, { signal });
      const stop = () => {
        window.removeEventListener("message", listener);
        frame.remove();
      };
      signal.addEventListener("abort", stop, { once: true });
      return stop;
    },
  });
}
