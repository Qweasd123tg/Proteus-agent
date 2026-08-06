# AGENTS.md

Инструкции для агентов и контрибьюторов, работающих с этим репозиторием.

## Главный Инвариант

Проект является модульным каркасом:

```text
Core -> Contract -> Module Implementation
```

Core не должен знать детали конкретного поиска, памяти, модели, tools, policy, patch algorithm или renderer. Новая функциональность должна проходить через существующий slot или через явно добавленный contract.

Для всех реализаций одного slot действует дополнительный инвариант:

```text
authority(module) = authority(slot, invocation_context)
```

Права, host capabilities, config, cancellation, lifecycle и failure semantics
не должны зависеть от `module_id`, языка или происхождения реализации
(`builtin`, `dylib`, `process`). Целевая внешняя граница — единый process
protocol; план однократного перехода описан в
`docs/process-module-architecture.md`. Текущий dylib runtime является только
переходным implemented state, а не поверхностью для дальнейшего расширения.

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

## Workspace Layout

```text
crates/
    proteus-contracts/     - публичный crate: traits, DTO, canonical model, временный dylib ABI
    proteus-core/       - ядро: runtime, wiring, process/plugin adapters переходного периода, app-server
    proteus-process-host/ - утилитарный крейт: lifecycle persistent stdio child-процессов (framing, request/response, restart)
clients/
    web/                 - основной Leptos chat-клиент
    inspector/           - отдельный Leptos config/architecture-клиент
modules/
    reference/           - reference/dogfood implementations; не default и не привилегированный pack
        file-tools/          - полноразмерный tool-плагин (read/write/edit/list/grep)
        git-tools/           - read-only git_status/git_diff tool-плагин
        shell-tool/          - tools shell / exec_command / write_stdin (sh -lc, PTY-сессии)
        plan-tool/           - tool update_plan (пошаговый план в transcript)
        rg-search/           - SearchBackend на ripgrep под id "rg"
        direct-patch/        - PatchApplier internal patch format под id "direct"
        sqlite-memory/       - MemoryStore на SQLite FTS5 как dylib
        codex-compactor/     - HistoryCompactor под id "codex"
        codex-tool-exposure/ - ToolExposure под id "codex_dynamic"
        coding-workflow/     - Workflow-плагины под ids "coding.single_loop", "coding.codex_loop" и "coding.plan_execute_review"
        context-pack/        - ContextBuilder-плагины под ids "simple", "repo_aware" и "codex_context"
        skill-pack/          - docs-on-disk skills: context provider "skills" + tool "skill"
        rust-lsp/             - tool lsp_diagnostics: Rust/rust-analyzer через persistent stdio LSP
        memory-pack/         - MemoryStore "jsonl"
        policy-pack/         - ApprovalPolicy плагины "allow_all", "ask_write", "codex_policy", "opencode_policy" + tool request_permissions
        renderer-pack/       - Renderer плагин "statusline"
    research/            - нестабилизированные module experiments вне production path
configs/                 - packaged named configs и prompts (источник install.sh)
examples/
    configs/             - example-профили (proteus.*.example.toml, config.example.json)
    modules/             - runnable process-module protocol examples
    mcp/                 - локальный smoke-test MCP server
    research/            - tracked заметки по upstream агентам
```

До cutover reference crates всё ещё собираются как dylib и устанавливаются в
текущий release bundle ради работоспособности dogfood-профилей. Это переходная
механика, описанная в `docs/dylib-transition.md`, а не standard pack и не
образец для новых модулей. Целевой process contract и naming описаны в
`docs/process-module-architecture.md`.

## Что Нельзя Ломать

- Не связывать модули напрямую друг с другом.
- Не добавлять новые dylib registrations, builtin concrete modules или
  origin-specific capabilities. Сначала мигрировать соответствующий slot на
  единый process contract.
- Не делать исключения по конкретному `module_id`: host dispatch разрешает
  методы по slot contract, а не по имени реализации.
- Не импортировать provider-specific типы OpenAI, Anthropic или локальных API за пределами `crates/proteus-core/src/adapters` и model shaping слоя.
- Не добавлять runtime-логику в CLI, если она принадлежит `core` или `workflow`.
- Не обходить `ToolRegistry`, `ApprovalPolicy` и `ToolSafety` при исполнении tools.
- Не менять DTO на границах модулей без обновления документации и тестов.
- Не превращать `docs/spec.md` в описание фактического состояния без явного разделения `implemented` и `planned`.
- Если модуль, профиль или workflow заявлен как копия/совместимый режим с
  Codex или другим upstream agent runtime, не добавляйте творческие fallback-и,
  эвристики или "улучшения" в той же реализации. Поведение, ошибки, stop
  conditions и failure paths должны повторять upstream настолько точно,
  насколько это позволяет текущий contract. Улучшения допускаются только как
  отдельный явно названный режим/module id/feature flag и должны быть
  задокументированы как divergence.

## Совместимость До Стабилизации

Проект находится в черновой pre-release фазе без внешних пользователей. Пока
владелец проекта явно не объявит текущие поверхности стабилизированными,
обратная совместимость для собственных config/API/DTO/wire/storage/plugin ABI
форматов не является целью.

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
2. Проверить, мигрирован ли slot на protocol v1 из
   `docs/process-module-architecture.md`.
3. Если да — реализовать внешний worker, не зависящий от `proteus-core`, и
   пройти conformance gate этого slot.
4. Если нет — сначала реализовать общий process adapter для всего slot. Не
   добавлять временный dylib/builtin путь для одной implementation.
5. Добавить explicit config/profile selection; reference implementation при
   необходимости разместить в `modules/reference/<name>`, не присваивая ей
   default/standard статус.
6. Добавить protocol и runtime swap evidence, затем обновить `docs/modules.md`
   и `docs/configuration.md`.

Во время перехода существующие dylib crates можно менять для bugfix или самой
миграции, но нельзя расширять их как будущий public ABI. Marketplace, package
manager, hot reload и sandbox не входят в process-only cutover.

## Как Добавлять И Проверять Фичу

Для существенного изменения используйте общий evidence path из
`docs/testing.md`:

1. Назовите измеримую проблему и ожидаемый проверяемый результат.
2. Разместите поведение в существующем contract/slot/tool/protocol boundary;
   новый slot сначала пропустите через `docs/slot-governance.md`.
3. Добавьте focused regression и применимый boundary/swap/protocol test.
4. Для runtime-поведения сохраните canonical journal evidence: поддерживаемый
   root `Success`/`Error` проверяйте через workflow replay, а внешний
   `Canceled`/`Timeout` — через `TurnSettled` и cold `/history`.
5. Replay используйте для проверки эквивалентности, dogfood/eval — для ответа
   «стало ли лучше»; намеренный divergence не обновляйте вслепую.
6. Прогоните применимый полный gate, обновите ближайшую русскую документацию и
   сделайте отдельный commit.

Не каждая правка требует всех видов evidence. Выберите строку матрицы в
`docs/testing.md` по затронутой границе и явно укажите непройденную применимую
проверку.

## Документация

Документация проекта ведётся на русском. Имена кода, API, traits, modules и config keys остаются английскими.

При изменении поведения обновляйте ближайший документ (полный индекс —
`docs/README.md`):

- quickstart и CLI: `README.md`;
- архитектурные границы: `docs/architecture.md`;
- module slots: `docs/modules.md`;
- целевая process-module архитектура: `docs/process-module-architecture.md`;
- временный dylib implementation reference: `docs/dylib-transition.md`;
- config schema и examples: `docs/configuration.md`;
- event log, sessions, REPL: `docs/runtime-and-events.md`;
- tools и approval: `docs/security-and-policy.md`;
- тестовые правила: `docs/testing.md`;
- vision/spec: `docs/spec.md`;
- roadmap: `docs/roadmap.md`;
- межпаковые контракты: `docs/pack-contracts.md`;
- research-черновики и архивы: `docs/research/`.

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
закрыты, какие отложены и почему. Отложенные идеи, UX-наблюдения и будущие
задачи фиксируйте в ближайшем подходящем markdown-документе (`docs/roadmap.md`,
`docs/spec.md`, профильный документ в `docs/` или отдельный
research/notes doc), чтобы их можно было закрыть позже.

## Проверка Перед Завершением

После успешной проверки изменений сразу фиксируйте их отдельным git commit,
если пользователь явно не попросил оставить рабочее дерево без коммита.

Минимум для документационных правок:

```bash
cargo test
```

Если менялась только документация и тесты не запускались, явно укажите это в финальном ответе.

Для архитектурных правок проверьте, что `tests/module_swap.rs` продолжает подтверждать заменяемость slots и canonical model contract.

Web-клиенты (`clients/web`, `clients/inspector`) исключены из root workspace и
собираются через Trunk: валидируйте их `trunk build` (не `cargo check` — он
может врать из-за lock), `trunk serve` слушает 1420/1421.
