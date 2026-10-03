import { form } from "./preferences.js";
import { requestNotificationPermission } from "../notify.js";
export function mount({ root, services, signal }) {
  const ui = form(root, services["client.preferences"], signal);
  ui.select(
    "sendMode",
    "Отправка сообщения",
    "Shift+Enter всегда переносит строку. Во время ответа отправка добавляет сообщение в очередь.",
    [
      ["enter", "Enter"],
      ["ctrl-enter", "Ctrl / Cmd + Enter"],
    ],
  );
  ui.toggle(
    "toolCardsCollapsed",
    "Компактные цепочки инструментов",
    "Сворачивать новые цепочки в одну строку. Уже раскрытые цепочки сохраняют свой вид.",
  );
  ui.toggle(
    "autoScroll",
    "Автопрокрутка",
    "Следовать за ответом. Прокрутка вверх приостанавливает следование. При отключении кнопка «К последнему сообщению» перемещает вниз один раз.",
  );
  const notifications = ui.toggle(
    "notifications",
    "Уведомления",
    "Когда окно не в фокусе: агент закончил, ждёт подтверждения или задал вопрос. Нажатие открывает чат.",
  );
  // Browsers allow the permission prompt only from a user action.
  notifications.addEventListener(
    "change",
    () => notifications.checked && requestNotificationPermission(),
    { signal },
  );
  ui.refresh();
}
