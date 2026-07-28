# Pack Contracts (черновик)

Статус: research-заметка. Это инвентарь неявных контрактов между плагинами
(packs) и направления, как снижать связанность. Документ дорабатывается по мере
находок; он не вводит новых правил сам по себе.

## Мотивирующий кейс

Codex pack: `codex-compactor` бережно сохраняет user-message
`<environment_context>` при компакции (parity-код из upstream Codex), но в
стеке долго не было producer-а этого блока — ни один context builder его не
эмитил. Модель не знала OS/shell и периодически галлюцинировала Windows `cmd`
на Linux. Consumer без producer-а никем не detected: связка жила только в
строковом префиксе.

Вывод: при сборке пака по чужому agent-shape consumer-логика копируется легко,
а producer-обязанность теряется молча. Такие связки нужно фиксировать явно.

## Инвентарь неявных контрактов

Форма связи почти всегда — строка (префикс текста, `name`, metadata key),
проходящая через ABI/JSON границу без compile-time проверки.

| Контракт | Producer | Consumer | Форма |
| --- | --- | --- | --- |
| `<environment_context>` блок | `context-pack` provider `environment` | model adapters (verbatim render), `codex-compactor` (`is_generated_user_message`) | константа `ENVIRONMENT_CONTEXT_TAG` в contracts |
| `<turn_aborted>` | нет (parity с upstream, producer отсутствует) | `codex-compactor` | префикс текста |
| `# AGENTS.md instructions` | `context-pack` provider `project_instructions` в `codex_context` | model adapters (verbatim render), `codex-compactor` | upstream-shaped текстовый envelope |
| summary prefix (`SUMMARY_PREFIX`) | `codex-compactor` | `codex-compactor` | префикс текста (само-согласован, ок) |
| `message.name == "context"` | `coding-workflow` | `codex-compactor`, `coding-workflow/history.rs`, token accounting | константа `CONTEXT_MESSAGE_NAME` в contracts |
| context metadata `model_visible_render = "verbatim"` | `context-pack` (`codex_context`) | OpenAI/Anthropic model adapters | `CONTEXT_RENDER_MODE_*` в contracts |
| chunk source `repo_aware:*` / `codex_context:*`, metadata `provider`/`reason`/`context_profile` | `context-pack` | app-server `context_map`, UI/debug views | строковые префиксы и metadata keys |
| tool metadata `hot`, `category`, `tags`, `aliases` | tool packs и `[tools.configured]` в config | `codex-tool-exposure` (`metadata_hot`) | metadata JSON у tool spec |
| `always_include` | named config | `codex-tool-exposure` | имена tools; `proteus doctor` warn-ит на неизвестные. В codex profile collaboration-имена валидны только при `subagents.surface = "collaboration"` |
| `PROTEUS_SHELL_SANDBOX=1` | operator environment | `shell-tool` | единый process-level opt-in workspace sandbox; model args не могут ослабить режим |
| `<shell>sh</shell>` в environment chunk | `context-pack` | согласовано с `shell-tool` (`sh -lc`) | константа `EXEC_SHELL` в contracts |
| `<available_skills>` + tool `skill {name}` | `skill-pack` context provider / named config | model prompt + `skill-pack` tool | provider id `skills`, tool name `skill`, project-over-user catalog lookup |
| `lsp_diagnostics` → `ContentLengthFraming` | `rust-lsp` tool / named config | `proteus-process-host`, `rust-analyzer` | tool name, `RunsCommands`, LSP initialize + didOpen/didChange + publishDiagnostics |
| request metadata `tool_exposure` (telemetry селектора) | `coding-workflow` (`request_from_state`) | usage snapshots, event log, UI debug views | metadata key у `CanonicalModelRequest` |
| structural shape и tool surface `CanonicalModelResponse` | model adapters | `ModelService`, workflow plugins, sequential subagent | общие contract helpers `validate_model_response_structure` / `validate_model_response_against_request`: assistant role, finish reason/tool consistency, ordered message projection, unique call ids, exact function/freeform round-trip для объявленного tool |
| `CanonicalModelResponse.end_turn` | model adapter (`openai.responses`) | strict `coding.codex_loop`, sequential subagent | optional canonical field; `false` требует следующий model round без provider-specific parsing в consumer-е |
| `ToolCall.raw_arguments` | model adapter (`openai.responses`) | tool orchestrator, request replay | optional исходная строка function arguments; является source of truth для parsed execution args, сохраняет malformed payload для failed tool output и следующего sampling round |
| прогресс/финал-структура ответа | `configs/prompts/opencode-default.md` | web-клиент рендерит транскрипт | текст промпта, контракта нет (полагаемся на модель) |

## Почему так

Строки и metadata через ABI-границу — сознательный trade-off: dylib-плагины не
могут делить rich types без стабилизации ABI, а `proteus-contracts` должен
оставаться узким. Проблема не в самих строках, а в том, что пары
producer/consumer нигде не перечислены и не проверяются.

## Направления снижения связанности

Отсортировано от дешёвого к дорогому; начинать с первых.

1. **Инвентарь (этот документ).** Любая новая межпаковая связка добавляется в
   таблицу выше. При сборке нового пака (opencode) — сначала выписать все
   consumer-ожидания, затем найти/создать producer-а для каждого.
2. **[сделано] Константы в `proteus-contracts`.** Маркеры, которые используют
   несколько crates, живут в `proteus_contracts::domain::markers`:
   `CONTEXT_MESSAGE_NAME` (`coding-workflow` ↔ `codex-compactor`),
   `ENVIRONMENT_CONTEXT_TAG` (`context-pack` ↔ `codex-compactor`),
   `CONTEXT_RENDER_MODE_*` (`context-pack` ↔ model adapters),
   `EXEC_SHELL` (`shell-tool` ↔ `context-pack`). Это не меняет ABI и убирает
   дрейф написания; связка проверяется компилятором через общий crate.
3. **[сделано] Проверки в `proteus doctor`.** Doctor warn-ит на имена tools в
   `module_config.tool_exposure.*.always_include` и subagent role
   tool-lists, которых нет в собранном tool registry. Имена
   `<server>__*` пропускаются, если MCP server сконфигурирован (discovery
   может быть недоступен при doctor run). Ловит опечатки и мёртвые записи
   после переименований.
4. **Pack-pair тесты.** Focused-тесты на связку в named config: например,
   «`codex_context` эмитит `<environment_context>` chunk, и
   `codex-compactor` сохраняет его при компакции». Тест живёт рядом с
   consumer-ом и падает, если producer пропал из профиля. Частично покрыто
   общими константами из п.2: producer и consumer тестируют один маркер.
5. **Декларации в `plugin.toml` (позже, если 1–4 не хватит).** Плагин
   декларирует `produces`/`consumes` списком contract ids; loader/doctor
   сверяет пары для активного профиля и предупреждает о consumer-ах без
   producer-ов. Это самый тяжёлый вариант — вводить только когда инвентарь
   покажет, что ручной учёт не масштабируется.
6. **Typed message origin (отдельное решение).** Сниффинг префиксов текста в
   compactor-е — следствие того, что у `CanonicalMessage` нет поля
   «происхождение» (`user | generated:context | generated:summary | ...`).
   Если появится второй compactor с той же логикой — рассмотреть typed
   поле/enum в contracts вместо префиксов. До этого не трогать: одно
   использование не оправдывает расширение DTO.

## Не делать

- Не типизировать все metadata keys подряд: string metadata остаётся ABI
  trade-off (см. `slot-governance.md`).
- Не строить validation framework до того, как doctor-проверки и pack-pair
  тесты покажут свои пределы.
- Не блокировать загрузку профиля из-за несшитых пар: сначала warnings,
  видимость важнее строгости.
