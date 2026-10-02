import { logicallyVisible, watchLogicalVisibility } from "./visibility.js";

const titles = {
  usage: "Расход и контекст",
  analysis: "Анализ ходов",
  configs: "Сборка агента",
  architecture: "Архитектура",
};
export function diagnosticsService(readUrl, subscribe, signal) {
  return Object.freeze({
    mount(view, root) {
      signal.throwIfAborted();
      if (!Object.hasOwn(titles, view))
        throw Error("Неизвестный раздел диагностики");
      const frame = document.createElement("iframe");
      frame.className = "diagnostic-frame";
      frame.title = titles[view];
      const desiredUrl = () => {
        const connection = readUrl();
        if (!connection) throw Error("Диагностика ещё не подключена");
        const next = new URL(connection, location.href);
        next.searchParams.set("embedded", "true");
        next.searchParams.set("view", view);
        return next;
      };
      let url,
        desired = desiredUrl(),
        stopped = false;
      const apply = () => {
        if (stopped || desired.href === url?.href || !logicallyVisible(root))
          return;
        url = desired;
        frame.src = url.href;
      };
      const refresh = () => {
        if (stopped) return;
        desired = desiredUrl();
        apply();
      };
      root.append(frame);
      const stopVisibility = watchLogicalVisibility(root, (visible) => {
        if (visible) apply();
      });
      const unsubscribe = subscribe(refresh);
      const listener = (event) => {
        if (
          stopped ||
          !url ||
          event.source !== frame.contentWindow ||
          event.origin !== url.origin ||
          event.data?.type !== "proteus-open-chat"
        )
          return;
        if (typeof event.data.href !== "string") return;
        let target;
        try {
          target = new URL(event.data.href, url);
        } catch {
          return;
        }
        const desktop =
          target.protocol === "proteus-desktop:" && target.pathname === "chat";
        if (!desktop && !["http:", "https:"].includes(target.protocol)) return;
        const current = new URL(location.href);
        const session =
          target.searchParams.get("session_dir") ||
          url.searchParams.get("session_dir");
        const sameServer =
          (target.searchParams.get("server") ??
            url.searchParams.get("server")) ===
          desired.searchParams.get("server");
        if (
          (desktop || target.origin === current.origin) &&
          sameServer &&
          session === desired.searchParams.get("session_dir")
        ) {
          document.dispatchEvent(
            new CustomEvent("proteus-client-navigation", { detail: "chat" }),
          );
          return;
        }
        // Preserve the app connection when crossing sessions.
        const chat =
          desktop || target.origin === current.origin ? current : target;
        chat.pathname = desktop ? "/index.html" : target.pathname;
        chat.hash = target.hash;
        for (const [key, value] of target.searchParams)
          chat.searchParams.set(key, value);
        if (session) chat.searchParams.set("session_dir", session);
        chat.searchParams.delete("settings_module");
        chat.searchParams.set("workspace_view", "chat");
        location.href = chat.href;
      };
      window.addEventListener("message", listener, { signal });
      const stop = () => {
        if (stopped) return;
        stopped = true;
        stopVisibility();
        unsubscribe();
        window.removeEventListener("message", listener);
        signal.removeEventListener("abort", stop);
        frame.remove();
      };
      signal.addEventListener("abort", stop, { once: true });
      return stop;
    },
  });
}
