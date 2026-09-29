import { form } from "./preferences.js";
export function mount({ root, services, signal }) {
  const service = services["client.preferences"];
  const ui = form(root, service, signal);
  ui.range("fontSize", "Размер текста", "От 12 до 22 пикселей", 12, 22, 1);
  ui.range(
    "chatWidth",
    "Ширина диалога",
    "Максимальная ширина сообщений и поля ввода.",
    420,
    1600,
    10,
  );
  ui.toggle(
    "animations",
    "Анимации",
    "Плавные переходы и раскрытие панелей. Системное уменьшение движения также учитывается.",
  );
  const preview = document.createElement("div");
  preview.className = "appearance-preview";
  preview.innerHTML =
    "<strong>Пример сообщения</strong><p>Так будет выглядеть текст в диалоге. Размер можно подобрать под свой экран.</p><code>let result = agent.run(task);</code>";
  root.append(preview);
  const refresh = () =>
    (preview.style.fontSize = `${service.read().fontSize}px`);
  const stop = service.subscribe(refresh);
  ui.refresh();
  refresh();
  return stop;
}
