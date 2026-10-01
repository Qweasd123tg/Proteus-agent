# AGENTS.md

Инструкции для агентов и контрибьюторов, работающих с этим репозиторием.

## Актуальный Контекст

Для выбора следующей работы используйте `docs/product/spec.md`,
`docs/product/scope.md` и `docs/product/roadmap.md`. Для конкретной правки
читайте документы по затронутой границе; индекс — `docs/README.md`.
`docs/archive/` и `examples/research/` исключайте из обычного поиска
контекста; они нужны только по явному запросу истории конкретного решения.

Текущий сценарий — локальная работа владельца с собственными доверенными
модулями. Sandbox и hardening для недоверенных сторонних модулей не являются
целью или блокером этого этапа. Не возвращайте их в обзор рисков, roadmap или
ответ «что дальше» без отдельного запроса владельца на эту тему. Это решение
о приоритетах, а не поручение отключать существующие policy/approval contracts.

## Главный Инвариант

Проект является модульным каркасом:

```text
Core -> Contract -> Module Implementation
```

Core не должен знать детали конкретного поиска, памяти, модели, tools, policy или patch algorithm. Новая функциональность должна проходить через существующий slot или через явно добавленный contract.

Для всех реализаций одного slot действует дополнительный инвариант:

```text
authority(module) = authority(slot, invocation_context)
```

Права, host capabilities, config, cancellation и failure semantics не должны
зависеть от `module_id`, языка или расположения реализации. Внешняя граница —
Component Runtime v2 с wire protocol v3, описанный в
`docs/architecture/process-module-architecture.md`. Dylib ABI, loader и wire-v2 session
удалены; возвращать второй native extension path или compatibility reader
нельзя.

Один процессный модуль может предоставлять несколько exports `slot/module_id`.
`component` — техническая запись его запуска в текущем config/runtime; все её
exports делят process lifecycle/failure domain. Authority всё равно
вычисляется по активному export, а не объединяется на component. Multiplexed
broker допускает concurrent invocation и host-routed reentrancy между exports
одного component; direct cross-export dispatch и union authority запрещены.

Transport и cardinality не смешиваются. Host-defined process contract явно
задаёт один из режимов:

```text
composition(contract) = select_one | ordered_many
```

Behavior slots используют `select_one`, кроме typed `hook/v2` chain с
явным порядком `modules.hooks`. `ordered_many` допустим только для
typed chain surface с одинаковой authority всех участников, явным порядком,
повторной validation и отдельным slot-governance evidence. Module не может
сам объявить новый hook или изменить composition mode.

Process boundary сам по себе не sandbox. Пока нет uniform launch policy,
таблица slot authority доказывает равенство protocol-visible `host.*` прав, но
модули остаются доверенными executable с OS-правами пользователя. Полный
инвариант включает одинаковый класс filesystem/network/env/process/resource
ограничений, когда такая sandbox surface появится.

## Модульность Кода

Модульность проекта должна отражаться и в структуре файлов. Не допускайте
накопления "жирных" файлов, где смешаны wiring, runtime flow, parsing,
rendering, UI state, tests и provider/module-specific детали.

Практические правила:

- Новый код добавляйте в маленький связный модуль, если это не ломает локальные
  conventions crate-а или клиента.
- Если файл уже выглядит крупным или смешивает несколько ответственностей,
  сначала ищите безопасный разрез: `builder`, `types`, `state`, `helpers`,
  `render`, `tests`, slot-specific adapter или feature-specific подмодуль.
- Ориентир: после изменения обычный production-файл должен оставаться
  обозримым. Если файл приближается к 500-700 строкам, дальнейшие добавления
  требуют явной причины; если он перевалил за это и вы его трогаете, сначала
  рассмотрите выделение связного блока.
- Не выносите код механически ради числа строк: модуль должен иметь понятную
  ответственность, стабильное имя и не создавать циклическое знание между
  слоями.
- Тесты можно держать рядом с кодом для локального поведения, но большие
  integration/swap/regression сценарии должны жить в отдельных test-модулях или
  `tests/`, чтобы production-файлы не превращались в свалку.
- UI-клиенты подчиняются тому же правилу: крупные страницы дробите на
  компоненты, состояние, transport/api bindings и view helpers, не смешивая их
  в одном `app.rs`.
- Contracts, process adapters и module helpers оформляйте по
  [единому образцу](docs/architecture/modules.md#единый-образец-реализации):
  сначала определите владельца общей ответственности. Одинаковый pack или
  похожий код не являются основанием связывать разные implementations.
  Повтор небольшого кода в самостоятельных modules допустим; канонические
  DTO и общие helpers должны сохранять границы и slot-specific semantics.

## Workspace Layout

- `crates/proteus-contracts` — публичные traits, DTO и canonical model;
  `proteus-core` — runtime, wiring, process adapters и app-server;
  `proteus-module-protocol` — component-v3 broker и conformance;
  `proteus-process-host` — lifecycle stdio процессных модулей.
- `clients/web`, `clients/inspector`, `clients/desktop` — Leptos-клиенты и
  Tauri-оболочка.
- `modules/reference` — reference implementations; каталог slots и exports
  описан в `docs/architecture/modules.md`. `modules/research` — эксперименты
  вне production path.
- `configs` — поставляемые profiles и prompts, источник для `install.sh`.
- `examples/configs`, `examples/modules`, `examples/mcp` — примеры config,
  внешних модулей и локального MCP server.

Reference crates линкуются только внутрь `proteus-reference-module` и не
являются отдельным runtime ABI. Installer публикует `proteus` и этот модуль в
одном release, но любой внешний executable с тем же process contract имеет
ровно тот же статус. Reference каталог не является standard/default pack.

## Что Нельзя Ломать

- Не связывать модули напрямую друг с другом.
- Не возвращать dylib registrations, in-process extension ABI, новые builtin
  concrete modules или origin-specific capabilities. Сначала мигрировать
  соответствующий slot на единый process contract.
- Не делать исключения по конкретному `module_id`: host dispatch разрешает
  методы по slot contract, а не по имени реализации.
- Не выдавать implementation дополнительные права из-за того, что она
  зарегистрировала tool через broad extension/hook path; одинаковая behavior
  surface должна проходить один contract и safety path.
- Не импортировать provider-specific типы OpenAI, Anthropic или локальных API за пределами provider implementation в `modules/reference/model-pack/src/adapters`. Core model shaping остаётся provider-neutral.
- Не добавлять runtime-логику в CLI, если она принадлежит `core` или `workflow`.
- Не обходить `ToolRegistry`, `ApprovalPolicy` и `ToolSafety` при исполнении tools.
- Не менять DTO на границах модулей без обновления документации и тестов.
- Не превращать `docs/product/spec.md` в описание фактического состояния без явного разделения `implemented` и `planned`.
- Если модуль, профиль или workflow заявлен как копия/совместимый режим с
  Codex или другим upstream agent runtime, не добавляйте творческие fallback-и,
  эвристики или "улучшения" в той же реализации. Поведение, ошибки, stop
  conditions и failure paths должны повторять upstream настолько точно,
  насколько это позволяет текущий contract. Улучшения допускаются только как
  отдельный явно названный режим/module id/feature flag и должны быть
  задокументированы как divergence.
  Различия настраиваемых instructions, profile и окружения сами по себе не
  означают дефект runtime: фиксируйте условия выбранной сборки и проверяйте
  конкретное расхождение поведения, а не требуйте одинакового текста промптов.

## Совместимость До Стабилизации

Проект находится в черновой pre-release фазе без внешних пользователей. Пока
владелец проекта явно не объявит текущие поверхности стабилизированными,
обратная совместимость для собственных config/API/DTO/wire/storage форматов
не является целью.

Практические правила:

- При изменении чернового контракта обновляйте все tracked producers,
  consumers, configs, tests и документацию в том же изменении, а старый путь
  удаляйте полностью.
- Не добавляйте migration shims, legacy aliases, deprecated fields/variants,
  dual-read/dual-write форматы, ABI tombstones, автоматическое распознавание
  старой формы или speculative fallback "на всякий случай".
- Не исправляйте устаревший input молча. Неизвестная config/API/wire форма
  должна завершаться явной ошибкой, чтобы черновой контракт можно было менять
  и упрощать без скрытых веток.
- Уже существующую pre-release compatibility не сохраняйте только потому, что
  она существует: при работе в соответствующем слое удаляйте её вместе со
  старыми тестами и оговорками в документации.
- Исключение требует отдельного явного решения владельца проекта с указанной
  границей совместимости. Точное повторение поведения upstream в специально
  названном compatible/parity режиме регулируется предыдущим разделом и не
  считается совместимостью со старыми версиями Proteus.
- Рабочие defaults, retry/error recovery и fallback-и текущего контракта не
  являются legacy автоматически. Удаляйте их только если исчезла сама
  актуальная семантика, а не по совпадению слова `fallback`.

## Как Добавлять Модуль

1. Найти подходящий trait в `crates/proteus-contracts/src/contracts`.
2. Проверить, имеет ли slot component export contract из
   `docs/architecture/process-module-architecture.md`.
3. Если да — реализовать внешний процессный модуль, не зависящий от
   `proteus-core`, и пройти conformance gate этого slot.
4. Если нет — сначала реализовать общий process adapter для всего slot. Не
   добавлять временный native/builtin путь для одной implementation.
5. Добавить explicit component export и config/profile selection; reference implementation при
   необходимости разместить в `modules/reference/<name>`, не присваивая ей
   default/standard статус.
6. Добавить protocol и runtime swap evidence, затем обновить
   `docs/architecture/modules.md` и `docs/guides/configuration.md`.

Model provider implementations проходят общий `model/v11` process contract.
Core владеет canonical shaping/validation, execution binding и journal, но не HTTP provider adapters. `AgentControl` — отдельный root-owned service для полных Proteus
peers, а не behavior slot или основание возвращать общий native loader.
Marketplace, package manager, hot reload и sandbox не входят в текущий process
runtime.

## Как Добавлять И Проверять Фичу

Для существенного изменения используйте общий evidence path из
`docs/development/testing.md`:

1. Назовите измеримую проблему и ожидаемый проверяемый результат.
2. Разместите поведение в существующем contract/slot/tool/protocol boundary;
   новый slot сначала пропустите через `docs/architecture/slot-governance.md`.
3. Используйте существующую проверку поведения; новый regression добавляйте,
   если она не ловит конкретный дефект. Boundary/swap/protocol test нужен при
   изменении соответствующей границы.
4. Для runtime-поведения сохраните canonical journal evidence: поддерживаемый
   root `Success`/`Error` проверяйте через workflow replay, а внешний
   `Canceled`/`Timeout` — через `TurnSettled` и cold `/history`.
5. Replay используйте для проверки эквивалентности, dogfood/eval — для ответа
   «стало ли лучше»; намеренный divergence не обновляйте вслепую.
6. Прогоните проверки по затронутой границе, обновите ближайшую русскую документацию и
   сделайте отдельный commit.

Не каждая правка требует всех видов evidence. Выберите строку матрицы в
`docs/development/testing.md` по затронутой границе и явно укажите непройденную применимую
проверку.

Не добавляйте тест для обратимой низкорисковой правки, если он только повторяет
implementation. Сначала расширяйте существующий boundary-сценарий. Один
инвариант проверяйте на нескольких слоях только тогда, когда каждый слой ловит
свой отдельный источник дефекта; совпадение assertions само по себе не является
основанием для ещё одного теста.

## Документация

Документация проекта ведётся на русском. Имена кода, API, traits, modules и config keys остаются английскими.

Обновляйте существующий документ по его ответственности. Промежуточные планы,
сводки прохода, гипотезы и завершённые этапы не добавляйте в текущие справочники.
Отдельный документ нужен только для самостоятельной долговечной темы.

При изменении поведения обновляйте ближайший документ (полный индекс —
`docs/README.md`):

- quickstart и CLI: `README.md`;
- архитектурные границы: `docs/architecture/architecture.md`;
- module slots: `docs/architecture/modules.md`;
- целевая process-module архитектура:
  `docs/architecture/process-module-architecture.md`;
- config schema и examples: `docs/guides/configuration.md`;
- event log, sessions, REPL: `docs/guides/runtime-and-events.md`;
- tools и approval: `docs/guides/security-and-policy.md`;
- тестовые правила: `docs/development/testing.md`;
- vision/spec: `docs/product/spec.md`;
- roadmap: `docs/product/roadmap.md`;
- межпаковые контракты: `docs/architecture/pack-contracts.md`;
- текущая документация: по индексу `docs/README.md`; исторические материалы:
  `docs/archive/`.

## Ведение Запросов Пользователя

Если пользователь просит "продолжить работу", "посмотреть что дальше",
вернуться после pull/update или в целом не даёт конкретного поручения на
изменение кода, сначала восстановите контекст и коротко обсудите следующие
варианты. Не начинайте новую реализацию галопом: предложите 2-3 разумных
направления, укажите рекомендуемое и дождитесь явного подтверждения вроде
"го", "делай", "начинай". Исключение — пользователь прямо просит выполнить
конкретную правку, команду, тест или review.

Если пользователь прислал подробный запрос с несколькими фичами, багами или
идеями, сначала разложите его на короткий checklist и ведите выполнение по
пунктам. Нельзя молча закрывать только самый очевидный пункт и оставлять
остальные без статуса.

Если в текущем заходе делается только часть списка, явно скажите, какие пункты
закрыты, какие отложены и почему. Согласованную с владельцем отложенную задачу
фиксируйте кратко в подходящем существующем документе. Собственные гипотезы
и наблюдения не превращайте автоматически в backlog или отдельный notes doc.

## Проверка Перед Завершением

После успешной проверки изменений сразу фиксируйте их отдельным git commit,
если пользователь явно не попросил оставить рабочее дерево без коммита.

Объём проверки выбирайте по изменению, согласно `docs/development/testing.md`.
Используйте `scripts/test.py` с явным package/target/filter, либо `full`
для полного Rust gate. Core fixtures требуют свежий путь к reference-модулю,
который готовит runner; не запускайте вложенные Cargo builds из tests.
Для документации достаточно проверить содержание, ссылки и `git diff --check`.
Для prompts/config добавляйте проверку загрузки затронутого профиля. Локальная
правка требует затронутого test target; полный workspace — общих contracts,
runtime wiring, зависимостей или изменений нескольких взаимодействующих crates.
После успешной проверки повторяйте её только при новых изменениях или конкретной
неразрешённой проблеме. В финале указывайте фактически выполненные проверки.
После полного прогона и исправления только упавших tests/fixtures повторяйте
эти targets, а не весь workspace. Новая общая production-правка требует
применимого полного gate.

Для архитектурных правок проверьте, что `tests/module_swap.rs` продолжает подтверждать заменяемость slots и canonical model contract.

Web-клиенты (`clients/web`, `clients/inspector`) исключены из root workspace и
собираются через Trunk: валидируйте их `trunk build` (не `cargo check` — он
может врать из-за lock), `trunk serve` слушает 1420/1421.
