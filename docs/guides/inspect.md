# Inspect

`inspect` показывает два связанных read-only представления:

- `plan` — что config просит собрать до запуска workers;
- `topology` — catalog/tool graph, полученный из того же плана.

Оба принадлежат Core и не являются отдельными module slots.

## CLI

```bash
proteus --config codex inspect plan
proteus --config codex inspect plan --format json

proteus --config codex inspect topology
proteus --config codex inspect topology --format table
proteus --config codex inspect topology --format markdown
proteus --config codex inspect topology --format mermaid
proteus --config codex inspect topology --format runtime
proteus --config codex inspect topology --format map
```

`inspect plan` показывает точные slot selections, components, exports,
contract versions, разрешённые host callbacks, requested tools и проверки.
Статус `blocked` означает, что runtime с таким планом не будет собран. Команда
не подключает workers и не выполняет handshake; raw config, component args,
environment и provider secrets в JSON projection не попадают.

Форматы:

- plan `text` — короткий человекочитаемый чертёж;
- plan `json` — полная безопасная diagnostic projection;
- default/table — компактные slots/modules/tools/warnings;
- markdown — переносимый отчёт;
- mermaid — полный diagnostic graph;
- runtime — короткий фактический turn path;
- map — человекочитаемая карта wiring.

Команда строит catalog и tool surface, но не отправляет model request.
Process components/exports валидируются; worker handshake выполняется там, где
нужна реальная registry/tool сборка.

## HTTP

App-server публикует:

- `GET /analysis?session_dir=<absolute-path>[&turn_id=<uuid>]` — сохранённые
  ходы с деталями выбранного хода из canonical journal;
- `GET /inspect/plan` — текущий JSON `AssemblyPlan`;
- `GET /inspect/topology` — JSON `TopologySnapshot`;
- `GET /inspect/topology.mmd` — полный Mermaid graph;
- `GET /inspect/topology.runtime` — короткий runtime path;
- `GET /inspect/topology.runtime.mmd` — короткая Mermaid runtime-схема;
- `GET /inspect/topology.map` — текстовая карта.

При token auth endpoints требуют тот же session token, что и остальные
app-server routes.

## Snapshot

`TopologySnapshot` содержит:

- profile, cwd, config path и expanded config files;
- `module_epoch`;
- permission mode;
- active model provider/name/stream;
- 9 core behavior slots отдельно от ordered-many context providers и tool
  registry;
- catalog modules;
- registered/enabled tools;
- graph edges;
- warnings.

Module source:

```text
builtin | process | config | unknown
```

- `process` — export из `[components.<id>.exports...]`;
- `builtin` — host-owned структурные contributions; не путь регистрации model implementation;
- `config` — config-defined runtime contribution;
- `unknown` — selected id, которого нет в catalog.

Отсутствующий slot не создаёт synthetic module с id `none` или `default`.
`active_module = null` прямо означает отсутствие selection.

## Tools

Tool node показывает:

- name, description и JSON schema;
- `ToolSafety`;
- source;
- registered/enabled state.

Process tools имеют source `dynamic/process-module`. То, что worker вернул
tool из `list`, ещё не делает его enabled: model-visible surface определяется
`tools.enabled`, policy и tool exposure.

В конструкторе сборки флаг `runtime_managed` отдельно от `enabled` показывает,
что доступность инструмента задаёт runtime, а не список `tools.enabled`.
Такие инструменты отображаются включёнными с недоступным переключателем;
их имена не добавляются в `tools.enabled` при сохранении. Уже записанное
в config имя сохраняется как есть. Для остальных инструментов переключатель,
счётчик и фильтр «Включённые» отражают текущий черновик `tools.enabled`;
отсутствующие в runtime, но выбранные в config имена остаются видны как
недоступные. `runtime_managed` описывает способ управления состоянием и не
меняет права инструмента, lifecycle или policy.
После сохранения отключённый обычный tool покидает текущий runtime catalog;
его повторное включение требует правки `tools.enabled` в config, поскольку
конструктор показывает только зарегистрированные и уже выбранные имена.

## Edges

Graph различает:

- config -> active selection;
- slot -> active/available module;
- config -> enabled tool;
- tool registry -> registered tool;
- runtime dependencies между slots.

Нет native package origin. Topology graph остаётся contract/export projection:
каждый `slot/module_id` отражается как ordinary process source. Группировку
exports по общему launch/failure domain показывает read-only component section
страницы Configs (`GET /config`). `tool/reference.tools` не имеет особого
статуса.

## Warnings

Snapshot может сообщить:

- invalid active provider;
- несколько merged config files;
- unknown active module;
- error best-effort catalog/tool сборки;
- слишком широкий tool surface при отсутствии tool exposure selection.

Selected process failure при реальной сборке runtime остаётся hard error, а не
warning/fallback.

## Inspector

`clients/inspector` использует JSON snapshot и config summary. UI показывает:

- process components и exports;
- slot selections;
- module source;
- enabled tools;
- runtime graph.

Раздел «Архитектура» содержит краткие сведения о профиле, карту связей и
отдельный каталог слотов/инструментов. Карта строится из `/inspect/topology`
локально, без Mermaid renderer или CDN. Mermaid остаётся форматом экспорта.

На карте доступны режимы «Сборка», «Модули» и «Инструменты». Последние два
по умолчанию показывают выбранные модули и включённые зарегистрированные
инструменты; переключатель «Неактивные» раскрывает остальные записи.
Поиск охватывает все объекты: выбор результата открывает нужный режим и
центрирует объект. Нажатие на узел выделяет соседей и связи; справа показаны
свойства, схема инструмента и переходы к связанным объектам, включая скрытые
текущим режимом. Связи берутся из snapshot, а их расположение задаёт клиент.
Это карта устройства сборки, а не трасса исполнения отдельной сессии.

Перемещение — перетаскиванием фона или стрелками при фокусе на карте, масштаб —
колесом и кнопками «−»/«+», «Вписать» или клавиша `0` возвращают общий вид.
Серия движений мыши и событий колеса обновляет transform не чаще одного раза
за кадр; завершение перетаскивания применяет конечную позицию сразу. Подпись
масштаба меняется только при изменении процента. Удаление карты отменяет
ожидающие обновления и наблюдение размеров.
«Развернуть» открывает карту на всё окно; `Esc` закрывает этот режим либо
сбрасывает выделение и поиск. В узком окне свойства располагаются под картой.
Обновление snapshot пересоздаёт карту и сбрасывает её локальный выбор.
Reference worker не получает отдельной визуальной категории.

Проверка UI после сборки обоих клиентов: `node --test clients/inspector/tests/*.test.mjs`
и общий `python3 clients/web/tests/extensions_browser.py`. Browser fixture
использует настоящий app-server с локальным provider fixture и проверяет
переходы карта/каталог, выбор узлов, связи, поиск, масштаб, клавиатуру,
узкое и развёрнутое представления и повторное обновление.
Для отдельной проверки Inspector у того же сценария есть `--inspector-only`.

После изменения schema:

```bash
cargo test --workspace
(cd clients/inspector && env -u NO_COLOR trunk build)
```

## Анализ сохранённого хода

Раздел Inspector `/analysis` (desktop: `/inspector.html?view=analysis`)
показывает список сессий и ходов, итоговый ответ, причину завершения,
конфигурацию на момент запуска и последовательность шагов. Сводка показывает
число запросов и инструментов, ошибки, незавершённые шаги и сумму известных
токенов с покрытием usage по запросам. Выбор сессии
не вызывает `/resume`, не запускает workers и не меняет активный чат.

Источник — canonical journal, а не текущий config или live SSE. HTTP
`/analysis` требует абсолютный `session_dir`; без `turn_id` выбирает последний
записанный ход. Неизвестный или некорректный идентификатор возвращает ошибку.
Ответ `AppSessionAnalysis` содержит краткий список ходов, `revision` последней
записи журнала и подробности только выбранного хода. DTO находятся в
`proteus-contracts::app_protocol::analysis`.

Шаг модели раскрывает сохранённые инструкции, сообщения, инструменты,
ответ или ошибку, токены провайдера и полный canonical JSON. Запросы
compactor помечены отдельно. Это вход после подготовки Core, не сырой
HTTP-пакет провайдера; redaction журнала сохраняется. Отсутствие usage
не трактуется как нулевой расход.

Шаг инструмента объединяет запрос, подтверждение, решение и результат по
execution/call identity. Отсутствие результата не считается успешным
выполнением или доказательством отсутствия внешнего эффекта. Время шага
включает ожидание подтверждения. Сжатие истории показывает записанный отчёт.

Сессия и выбранный ход сохраняются в URL (`analysis_session`, `analysis_turn`).
Поиск фильтрует названия шагов; отдельный фильтр оставляет ошибки и шаги без
результата. «Обновить снимок» перечитывает журнал, автоматического live-потока
на этом экране нет. Статус «Нет завершения» означает отсутствие `TurnSettled`
в снимке и сам по себе не доказывает, что процесс сейчас работает.

Порядок шагов соответствует journal sequence и не является графом причинных
зависимостей. Отдельные журналы peers и внутренние HTTP-повторы провайдера
не объединяются. Сравнение профилей и сводный отчёт по нескольким сессиям
не входят в этот экран.

## Отличие От /config

`/config` — клиентская projection настроек и доступных UI choices.
`/inspect/plan` — неизменяемый чертёж текущего module epoch.
`/inspect/topology` — graph catalog-а и фактически зарегистрированных tools,
построенный из этого чертежа. Первый удобен для формы, второй — для ответа
«что хотим собрать», третий — «что подключено и почему».

Подробный contract плана:
[assembly-plan.md](../architecture/assembly-plan.md).
