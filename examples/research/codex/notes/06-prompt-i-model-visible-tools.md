# Как Codex строит prompt и model-visible tools

## Главная идея

В Codex prompt не хранится как готовая строка.

Он собирается конвейером:

`session config -> TurnContext -> ToolRouter -> build_prompt -> client.rs -> Responses API`

Это один из самых полезных кусков архитектуры для собственного агента.

## Шаг 1. Выбор базовых инструкций сессии

Ключевое место:

- `codex-rs/core/src/codex.rs`

При создании сессии Codex выбирает `base_instructions` по приоритету:

1. `config.base_instructions`
2. `conversation_history.get_base_instructions()`
3. стандартные инструкции выбранной модели с учетом `personality`

Это хороший дизайн, потому что:

- есть явный override;
- есть восстановление из истории;
- есть fallback на модельный default.

## Шаг 2. Сборка `SessionConfiguration`

В `SessionConfiguration` попадают:

- provider;
- collaboration mode;
- reasoning summary;
- service tier;
- developer instructions;
- user instructions;
- personality;
- base instructions;
- approval policy;
- sandbox policy;
- cwd;
- dynamic tools;
- session source.

То есть все важные стартовые параметры сессии собираются в один объект.

## Шаг 3. Переход к `TurnContext`

Когда начинается конкретный turn, Codex строит `TurnContext`.

Он содержит:

- модель и `model_info`;
- reasoning settings;
- sandbox/approval policies;
- `tools_config`;
- features;
- `personality`;
- output schema;
- `dynamic_tools`;
- metadata/timing state.

Именно `TurnContext` задает реальные условия одного model turn.

## Шаг 4. Сборка набора tools для этого turn

Ключевые файлы:

- `codex-rs/core/src/codex.rs`
- `codex-rs/core/src/tools/router.rs`
- `codex-rs/core/src/tools/spec.rs`

### Источники tool-ов

Во время сборки tool layer сходятся:

- MCP tools;
- app tools;
- discoverable tools;
- dynamic tools.

После этого:

- `ToolRouter::from_config(...)` строит итоговый router;
- router умеет отдать `model_visible_specs()`;
- handlers уже привязаны к registry.

То есть модель видит не весь внутренний runtime, а только итоговый, специально подготовленный список доступных tools.

### Отложенные dynamic tools

Если dynamic tool помечен `defer_loading`, он не попадает в видимый модели список сразу.

Это очень важный момент: Codex умеет различать:

- tool существует в runtime;
- tool должен быть прямо сейчас виден модели.

## Шаг 5. Сборка `Prompt`

Ключевые файлы:

- `codex-rs/core/src/client_common.rs`
- `codex-rs/core/src/codex.rs`

Структура `Prompt` содержит:

- `input`
- `tools`
- `parallel_tool_calls`
- `base_instructions`
- `personality`
- `output_schema`

А функция `build_prompt(...)` берет:

- уже собранный input;
- `router.model_visible_specs()`;
- `turn_context`;
- `base_instructions`

И возвращает итоговый внутренний prompt.

## Шаг 6. Что попадает в `input`

Содержимое `input` собирается не одной строкой, а из истории и контекстных обновлений.

Ключевое место:

- `build_initial_context(...)` в `codex-rs/core/src/codex.rs`

Там добавляются:

- model instruction updates;
- permission instructions;
- developer instructions;
- memory tool instructions;
- collaboration mode instructions;
- realtime update fragments;
- personality fragments;
- contextual user sections.

То есть Codex строит prompt как набор структурированных `ResponseItem`, а не как одну слепленную строку.

## Шаг 7. Переход в `client.rs`

Ключевой файл:

- `codex-rs/core/src/client.rs`

Именно здесь внутренний `Prompt` превращается во внешний запрос к модели.

Используются:

- `prompt.base_instructions.text`
- `prompt.get_formatted_input()`
- `create_tools_json_for_responses_api(&prompt.tools)`
- `prompt.parallel_tool_calls`
- `prompt.output_schema`

Дальше это упаковывается в `ResponsesApiRequest`.

## Практический смысл `parallel_tool_calls`

Это поле не вычисляется отдельно в tool router.

Оно берется из:

- `turn_context.model_info.supports_parallel_tool_calls`

То есть возможность параллельных tool calls зависит не только от дизайна Codex, но и от свойств конкретной модели.

Это тоже хороший архитектурный ход: модельные ограничения учитываются в prompt layer.

## Что особенно полезно для своего агента

Из этого слоя стоит заимствовать пять вещей:

1. Не хранить prompt как строку, а собирать его из структурированных элементов.
2. Разделять `SessionConfiguration` и `TurnContext`.
3. Строить tool set на каждый turn, а не держать один глобальный список.
4. Отделять runtime-существующие tools от model-visible tools.
5. Делать отдельный слой преобразования внутреннего prompt в внешний API request.

## Главный вывод

Codex строит prompt не "в момент отправки модели", а через цепочку зависимых слоев:

- история и context updates;
- session config;
- turn context;
- tool router;
- internal prompt;
- request builder.

Это одна из самых сильных частей архитектуры Codex.

## Что читать дальше

Лучший порядок:

1. `codex-rs/core/src/client_common.rs`
2. `codex-rs/core/src/codex.rs`
3. `codex-rs/core/src/tools/router.rs`
4. `codex-rs/core/src/tools/spec.rs`
5. `codex-rs/core/src/client.rs`
