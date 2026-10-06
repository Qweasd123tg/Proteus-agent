// Client wording for host-defined slots; unknown slots keep the server text.
export const slotText = {
  workflow: ["Рабочий цикл", "Управляет шагами агента от запроса до результата."],
  context: ["Контекст", "Собирает сведения, которые получает модель."],
  compactor: ["Сжатие истории", "Сокращает длинную историю без потери рабочего контекста."],
  tool_exposure: ["Выбор инструментов", "Определяет, какие инструменты видит модель."],
  policy: ["Подтверждения", "Решает, когда действие требует разрешения."],
  search: ["Поиск", "Ищет нужные сведения в рабочем проекте."],
  memory: ["Память", "Сохраняет и возвращает сведения между обращениями."],
  patch: ["Правки файлов", "Применяет подготовленные изменения к файлам."],
};

const changeText = {
  ...Object.fromEntries(Object.entries(slotText).map(([id, [title]]) => [id, title])),
  model: "Параметры модели",
  hook: "Обработчики",
  tools: "Инструменты",
  provider: "Модель",
  mode: "Режим прав",
};

export const changeLabel = (key) => changeText[key] ?? key;

// Tool safety classes as the user reads them; unknown classes keep their name.
export const safetyText = {
  ReadOnly: "только чтение",
  WritesFiles: "меняет файлы",
  RunsCommands: "запускает команды",
  Network: "сеть",
  Dangerous: "опасный",
};

export const permissionText = {
  plan: ["Только чтение", "Доступны только инструменты чтения."],
  normal: ["По правилам", "Решения принимает политика подтверждений; спорные действия ждут вашего ответа."],
  auto: ["Правки без вопросов", "Чтение и изменение файлов без подтверждения; команды и сеть запрещены."],
};
