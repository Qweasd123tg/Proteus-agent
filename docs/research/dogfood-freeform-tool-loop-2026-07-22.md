# Dogfood: цикл пустого `apply_patch` (2026-07-22)

Статус: blocker локализован и закрыт в runtime/config. Этот документ —
postmortem конкретного запуска, а не общий provider compatibility claim.

## Сценарий и наблюдение

Readiness dogfood запускал маленькую coding-задачу через web/app-server с
packaged `codex` profile и личным OpenAI-compatible proxy. В двух turn-ах
сохранилось 42 model request record-а и 40 completed responses. Provider
boundary вернула 17 вызовов `apply_patch` с function surface и пустой строкой
arguments; каждый
закончился `EOF while parsing a value`, после чего `coding.codex_loop`
продолжил sampling. Provider usage за запуск составил примерно 1,99 млн input
tokens (из них примерно 1,83 млн cached) и 16,8 тыс. output tokens.

Файлы целевого репозитория не изменились. Session/request snapshots позволили
локализовать failure, но raw SSE body не сохранялся, поэтому по имеющимся
данным нельзя честно разделить ошибку конкретной модели и преобразование
личного proxy.

## Причина

Request объявлял `apply_patch` как OpenAI Responses custom tool
(`type = "custom"`) с freeform grammar. Нормальный ответ для этой формы —
`custom_tool_call` с raw `input`. Граница model/proxy вместо этого вернула
`function_call` с пустыми `arguments`.

Proteus правильно сохранил фактически полученный function call, но не проверил,
что его surface совпадает с отправленной декларацией. Workflow поэтому увидел
обычный malformed tool call, вернул модели recoverable failed `ToolResult` и в
unbounded Codex-compatible loop не имел локального stop condition.

## Принятое исправление

1. В `ModelCapabilities` добавлена явная `supports_freeform_tools`, default —
   `false`.
2. `RequestShaper` отклоняет freeform tools до provider call, если capability
   не включена.
3. `ModelService` проверяет terminal response против фактически отправленного
   request. Function/freeform mismatch теперь является protocol error до
   изменения истории и исполнения tool-а.
4. Packaged proxy profiles `codex` и `glm` используют builtin function-style
   `apply_patch` с JSON-аргументом `patch`.
5. OpenAI adapter сохраняет настоящую custom-tool поддержку: проверенный
   endpoint/model может явно включить capability и использовать
   `ToolSurface::Freeform`. Silent fallback и угадывание wire shape не
   добавлялись.

OpenAI wire distinction сверялось с официальными разделами
[Using tools](https://developers.openai.com/api/docs/guides/tools) и
[Custom tools](https://developers.openai.com/api/docs/guides/function-calling#custom-tools).

## Не входит в этот фикс

Общий request/token budget для `coding.codex_loop` остаётся отдельной задачей.
Он ограничил бы ущерб от будущего неизвестного failure mode, но не исправил бы
нарушенный provider contract и изменил бы stop conditions Codex-compatible
workflow. Решение должно приниматься как отдельный явно названный divergence,
а не как скрытый fallback текущего режима.
