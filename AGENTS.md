# AGENTS.md

Инструкции для агентов и контрибьюторов, работающих с этим репозиторием.

## Главный Инвариант

Проект является модульным каркасом:

```text
Core -> Contract -> Module Implementation
```

Core не должен знать детали конкретного поиска, памяти, модели, tools, policy, patch algorithm или renderer. Новая функциональность должна проходить через существующий slot или через явно добавленный contract.

## Что Нельзя Ломать

- Не связывать модули напрямую друг с другом.
- Не импортировать provider-specific типы OpenAI, Anthropic или локальных API за пределами `src/adapters` и model shaping слоя.
- Не добавлять runtime-логику в CLI, если она принадлежит `core` или `workflow`.
- Не обходить `ToolRegistry`, `ApprovalPolicy` и `ToolSafety` при исполнении tools.
- Не менять DTO на границах модулей без обновления документации и тестов.
- Не превращать `docs/MODULAR_AGENT_SPEC_RU.md` в описание фактического состояния без явного разделения `implemented` и `planned`.

## Как Добавлять Модуль

1. Найти подходящий trait в `src/contracts`.
2. Реализовать модуль в подходящей подпапке `src/modules` или adapter в `src/adapters`.
3. Зарегистрировать строковый ключ, manifest и factory в `BuiltinModuleCatalog`.
4. Добавить или обновить конфиг-пример.
5. Добавить тест на заменяемость, если модуль относится к slot.
6. Обновить `docs/modules.md` и при необходимости `docs/configuration.md`.

Для v0 модульность означает выбор встроенной реализации через config. Динамическая загрузка, marketplace, WASM runtime и hot-reload не являются текущей целью.

## Документация

Документация проекта ведётся на русском. Имена кода, API, traits, modules и config keys остаются английскими.

При изменении поведения обновляйте ближайший документ:

- quickstart и CLI: `README.md`;
- архитектурные границы: `docs/architecture.md`;
- module slots: `docs/modules.md`;
- config schema и examples: `docs/configuration.md`;
- planned права tools/modules: `docs/rights-and-modules.md`;
- event log, sessions, REPL: `docs/runtime-and-events.md`;
- tools и approval: `docs/security-and-policy.md`;
- тестовые правила: `docs/testing.md`;
- vision/spec: `docs/MODULAR_AGENT_SPEC_RU.md`.

## Проверка Перед Завершением

Минимум для документационных правок:

```bash
cargo test
```

Если менялась только документация и тесты не запускались, явно укажите это в финальном ответе.

Для архитектурных правок проверьте, что `tests/module_swap.rs` продолжает подтверждать заменяемость slots и canonical model contract.
