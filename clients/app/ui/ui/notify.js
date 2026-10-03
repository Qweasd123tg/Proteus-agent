// Background chats report to the system. The desktop shell sends the
// notification over D-Bus; a browser uses the Notification API once allowed.
const shell = () => window.__TAURI__;

function open(sessionDir) {
  window.dispatchEvent(
    new CustomEvent("proteus-open-session", { detail: sessionDir }),
  );
}

export function showNotification(title, body, sessionDir) {
  if (document.hasFocus()) return;
  const invoke = shell()?.core?.invoke;
  if (invoke) {
    invoke("notify", { title, body, sessionDir }).catch((error) =>
      console.warn("Не удалось показать уведомление", error),
    );
    return;
  }
  if (!("Notification" in window) || Notification.permission !== "granted")
    return;
  const notice = new Notification(title, { body, tag: sessionDir });
  notice.onclick = () => {
    window.focus();
    notice.close();
    open(sessionDir);
  };
}

/** Clicks on shell notifications arrive as an app event. */
export function listenNotificationClicks() {
  const listen = shell()?.event?.listen;
  if (!listen) return () => {};
  const listening = listen("proteus-notification-open", (event) =>
    open(event.payload),
  );
  return () => listening.then((unlisten) => unlisten());
}

export function requestNotificationPermission() {
  if (shell() || !("Notification" in window)) return;
  if (Notification.permission === "default")
    Notification.requestPermission().catch(() => {});
}
