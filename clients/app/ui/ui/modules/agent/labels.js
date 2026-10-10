// Technical names stay English; explanations are client wording, not contracts.
// Unknown slots keep the server text.
export const slotText = {
  model: ["Model", "Модель профиля и параметры её модуля."],
  workflow: ["Workflow", "Управляет шагами агента от запроса до результата."],
  context: ["Context", "Собирает сведения, которые получает модель."],
  compactor: ["Compactor", "Сокращает длинную историю без потери рабочего контекста."],
  tool_exposure: ["Tool Exposure", "Определяет, какие инструменты видит модель."],
  policy: ["Policy", "Режим прав профиля и политика подтверждений."],
  search: ["Search", "Ищет нужные сведения в рабочем проекте."],
  memory: ["Memory", "Сохраняет и возвращает сведения между обращениями."],
  patch: ["Patch", "Применяет подготовленные изменения к файлам."],
  tool: ["Tools", "Разрешённые инструменты и их отбор для модели."],
  hook: ["Hooks", "Порядок и параметры обработчиков шагов агента."],
};

export const pluginText = ["Plugins", "Процессные компоненты, их пакеты инструментов, обработчики и модули слотов."];

// Settings page that selects a slot's module; plugin exports link to it.
export const slotPage = {
  model: "agent-model",
  workflow: "agent-workflow",
  context: "agent-context",
  compactor: "agent-compactor",
  tool_exposure: "agent-tools",
  policy: "agent-access",
  patch: "agent-patch",
  search: "agent-search",
  memory: "agent-memory",
  hook: "agent-hooks",
};

const changeText = {
  ...Object.fromEntries(Object.entries(slotText).map(([id, [title]]) => [id, title])),
  addons: "Дополнения агента",
  tools: slotText.tool[0],
  provider: slotText.model[0],
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
