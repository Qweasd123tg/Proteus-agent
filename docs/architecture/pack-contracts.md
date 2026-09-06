# Pack Contracts

Статус: живой справочный инвентарь неявных связей между modules, profiles и
prompts внутри pack-а. Документ обновляется рядом с изменением producer или
consumer, но сам по себе не вводит новый public contract.

Модули могут зависеть от общих markers, tool names и metadata.
При изменении producer нужно проверить соответствующий consumer.

## Инвентарь неявных контрактов

Часть связей — строки (префиксы текста, имена tools, metadata keys),
проходящие через JSON/process границу. Общие typed поля перечислены отдельно;
имена и текстовые envelopes не заменяют их структурную семантику.

| Контракт | Producer | Consumer | Форма |
| --- | --- | --- | --- |
| `<environment_context>` блок | `context-pack` provider `environment` | model-facing context render | текстовый envelope; компакторы определяют request context по scope, не по префиксу |
| `<turn_aborted>` | нет (parity с upstream, producer отсутствует) | `codex-compactor` | префикс текста |
| `# AGENTS.md instructions` | `context-pack` provider `project_instructions` в `codex_context` | model-facing context render | upstream-shaped текстовый envelope; не признак persistent/ephemeral history |
| summary prefix (`SUMMARY_PREFIX`) | `codex-compactor` | `codex-compactor` | префикс текста (само-согласован, ок) |
| `CanonicalPart.scope` | workflow/context construction | `codex-compactor`, Python `python_suffix`, `coding-workflow/history.rs` | непустое сообщение со всеми parts в `request` — ephemeral context; `message.name` не влияет на классификацию |
| context metadata `model_visible_render = "verbatim"` | `context-pack` (`codex_context`) | OpenAI/Anthropic model adapters | `CONTEXT_RENDER_MODE_*` в contracts |
| chunk source `repo_aware:*` / `codex_context:*`, metadata `provider`/`reason`/`context_profile` | `context-pack` | app-server `context_map`, UI/debug views | строковые префиксы и metadata keys |
| tool metadata `hot`, `category`, `tags`, `aliases` | tool packs и `[tools.configured]` в config | `codex-tool-exposure` (`metadata_hot`) | metadata JSON у tool spec |
| `always_include` / `allow` / `ask_before` / `deny` / `allow_sandboxed` списки | named config | `policy-pack`, `codex-tool-exposure` | имена tools; `proteus doctor` warn-ит на неизвестные. В codex profile collaboration-имена agent control валидны только при `agent_control.surface = "collaboration"` |
| `with_escalated_permissions` + `justification` | `shell-tool` (аргументы tool) | `policy-pack` (`allow_sandboxed`), core `tool_orchestrator` | имена аргументов tool call |
| `request_permissions` → `granted_permissions` | `policy-pack` tool | core `PolicyContext`, `policy-pack` при следующих вызовах | имя tool + семантика grant scope |
| `approval.cache_scopes = ["workspace_write"]` | builtin `ApplyPatchTool` metadata | core `approval/cache` | metadata key + имя scope |
| `<shell>sh</shell>` в environment chunk | `context-pack` | согласовано с `shell-tool` (`sh -lc`) | константа `EXEC_SHELL` в contracts |
| `<available_skills>` + tool `skill {name}` | `skill-pack` context provider / named config | model prompt + `skill-pack` tool | provider id `skills`, tool name `skill`, project-over-user catalog lookup |
| `lsp_diagnostics` → `ContentLengthFraming` | `rust-lsp` tool / named config | `proteus-process-host`, `rust-analyzer` | tool name, `RunsCommands`, LSP initialize + didOpen/didChange + publishDiagnostics |
| opencode `groups.*.tools` (маппинг tool → permission-группа) | named config `opencode` | `policy-pack` (`opencode_policy`) | имена tools; `proteus doctor` проверяет вложенные `tools`-списки |
| opencode `pattern_args` (`command`/`path`/`paths`) | named config `opencode` | `opencode_policy` читает эти ключи из `ToolCall.args` | имена аргументов tools из `shell-tool`/`file-tools`; при переименовании аргумента правила молча перестанут матчиться |
| request metadata `tool_exposure` (telemetry селектора) | `coding-workflow` (`request_from_state`) | usage snapshots, event log, UI debug views | metadata key у `CanonicalModelRequest` |
| structural shape и tool surface `CanonicalModelResponse` | model adapters | `ModelService`, workflow modules | общие contract helpers `validate_model_response_structure` / `validate_model_response_against_request`: непустой ordered `messages`, assistant role, finish reason/tool consistency, ordered tool-call projection, unique call ids, exact function/freeform round-trip для объявленного tool |
| `CanonicalMessage.phase` + ordered response item boundary | OpenAI Responses adapter | workflows, journal/transcript/replay, compactor, следующий model request | typed `MessagePhase::{Commentary, FinalAnswer}`; `None` для providers без классификации; consumers сохраняют все items, terminal output выбирает последнее непустое assistant message |
| `CanonicalModelResponse.end_turn` | model adapter (`openai.responses`) | strict `coding.codex_loop` | optional canonical field; `false` требует следующий model round без provider-specific parsing в consumer-е |
| `ToolCall.raw_arguments` | model adapter (`openai.responses`) | tool orchestrator, request replay | optional исходная строка function arguments; является source of truth для parsed execution args, сохраняет malformed payload для failed tool output и следующего sampling round |
| прогресс/финал-структура ответа | `configs/prompts/opencode-default.md` | web-клиент рендерит транскрипт | текст промпта, контракта нет (полагаемся на модель) |

## Проверка Связок

Общие markers находятся в `proteus_contracts::domain::markers`.
Это уменьшает дрейф написания, но не доказывает совместимость поведения.

`proteus doctor` предупреждает об неизвестных tool names в
`module_config.*` списках `allow`, `allow_sandboxed`, `ask_before`,
`deny`, `always_include` и вложенных `tools`.
Имена `<server>__*` допускаются для configured MCP server, когда discovery
недоступен при doctor run.

Для изменяемой связки проверяется, что producer действительно выдаёт
ожидаемый input, а consumer сохраняет нужное поведение в собранном profile.
Успешный handshake и совпадение констант сами по себе этого не доказывают.

`cargo test -p proteus-reference-worker --test compactor_interop` запускает
Rust и Python compactor через один process slot: переименование или отсутствие
имени request context не меняет retention, а пользовательское имя `context`
не превращает сообщение в служебное. Стратегии сжатия не обязаны совпадать.
