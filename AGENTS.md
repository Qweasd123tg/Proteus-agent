# AGENTS.md

Инструкции для агентов и контрибьюторов, работающих с этим репозиторием.

## Главный Инвариант

Проект является модульным каркасом:

```text
Core -> Contract -> Module Implementation
```

Core не должен знать детали конкретного поиска, памяти, модели, tools, policy, patch algorithm или renderer. Новая функциональность должна проходить через существующий slot или через явно добавленный contract.

## Workspace Layout

```text
crates/
    proteus-contracts/     - публичный crate: traits, DTO, canonical model, plugin ABI
    proteus-core/       - ядро: runtime, core wiring, plugin_adapters, stubs, adapters, app-server
clients/
    web/                 - основной Leptos chat-клиент
    inspector/           - отдельный Leptos config/architecture-клиент
plugins/
    default/             - стандартный набор плагинов и ABI-примеры
        file-tools/          - полноразмерный tool-плагин (read/write/list/grep)
        git-tools/           - read-only git_status/git_diff tool-плагин
        shell-tool/          - tool shell (sh -lc)
        rg-search/           - SearchBackend на ripgrep под id "rg"
        direct-patch/        - PatchApplier internal patch format под id "direct"
        sqlite-memory/       - MemoryStore на SQLite FTS5 как dylib
        codex-compactor/     - HistoryCompactor под id "codex"
        codex-tool-exposure/ - ToolExposure под id "codex_dynamic"
        coding-workflow/     - Workflow-плагины под ids "coding.single_loop", "coding.codex_loop" и "coding.plan_execute_review"
        context-pack/        - ContextBuilder-плагины под ids "simple", "repo_aware" и "codex_context"
        memory-pack/         - MemoryStore "jsonl" и MemoryPolicy "carry_forward"
        policy-pack/         - ApprovalPolicy плагины "allow_all", "ask_write" и "codex_policy"
        renderer-pack/       - Renderer плагины "plain" и "statusline"
```

Плагины живут в `~/.proteus/plugins/` и не зависят от `proteus-core`; граница проходит через `proteus-contracts` и ABI glue на `abi_stable`. Детали — `docs/plugin-architecture.md`.

## Что Нельзя Ломать

- Не связывать модули напрямую друг с другом.
- Не импортировать provider-specific типы OpenAI, Anthropic или локальных API за пределами `crates/proteus-core/src/adapters` и model shaping слоя.
- Не добавлять runtime-логику в CLI, если она принадлежит `core` или `workflow`.
- Не обходить `ToolRegistry`, `ApprovalPolicy` и `ToolSafety` при исполнении tools.
- Не менять DTO на границах модулей без обновления документации и тестов.
- Не превращать `docs/MODULAR_PROTEUS_SPEC_RU.md` в описание фактического состояния без явного разделения `implemented` и `planned`.
- Если модуль, профиль или workflow заявлен как копия/совместимый режим с
  Codex или другим upstream agent runtime, не добавляйте творческие fallback-и,
  эвристики или "улучшения" в той же реализации. Поведение, ошибки, stop
  conditions и failure paths должны повторять upstream настолько точно,
  насколько это позволяет текущий contract. Улучшения допускаются только как
  отдельный явно названный режим/module id/feature flag и должны быть
  задокументированы как divergence.

## Как Добавлять Модуль

1. Найти подходящий trait в `crates/proteus-contracts/src/contracts`.
2. Реализовать модуль как dylib-плагин в `plugins/default/<name>` для стандартного набора или в отдельном pack-каталоге вроде `plugins/experimental/<name>`; core-owned fallback размещать в `crates/proteus-core/src/stubs`, provider adapter — в `crates/proteus-core/src/adapters`, ABI glue нового plugin slot — в `crates/proteus-core/src/plugin_adapters`.
3. Зарегистрировать строковый ключ, manifest и factory в `BuiltinModuleCatalog`.
4. Добавить или обновить конфиг-пример.
5. Добавить тест на заменяемость, если модуль относится к slot.
6. Обновить `docs/modules.md` и при необходимости `docs/configuration.md`.

Альтернативно: модуль можно реализовать как отдельный dylib-плагин в `~/.proteus/plugins/`, depends только на `proteus-contracts`. См. `docs/plugin-architecture.md`.

Для v0 модульность означает либо выбор встроенной реализации через config, либо загрузку dylib-плагина. Marketplace, WASM runtime, hot-reload и sandbox не являются текущей целью.

## Документация

Документация проекта ведётся на русском. Имена кода, API, traits, modules и config keys остаются английскими.

При изменении поведения обновляйте ближайший документ:

- quickstart и CLI: `README.md`;
- архитектурные границы: `docs/architecture.md`;
- module slots: `docs/modules.md`;
- plugin ABI и waves: `docs/plugin-architecture.md`;
- config schema и examples: `docs/configuration.md`;
- event log, sessions, REPL: `docs/runtime-and-events.md`;
- tools и approval: `docs/security-and-policy.md`;
- тестовые правила: `docs/testing.md`;
- vision/spec: `docs/MODULAR_PROTEUS_SPEC_RU.md`;
- roadmap: `docs/roadmap.md`;
- memory plugin blueprint (research): `docs/memory-research.md`.

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
`docs/MODULAR_PROTEUS_SPEC_RU.md`, профильный документ в `docs/` или отдельный
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
