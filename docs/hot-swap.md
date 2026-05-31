# Hot-Swap И Dynamic Modules

Этот документ фиксирует planned boundary для будущей горячей замены модулей.
Фактическая реализация v0 пока статическая: config читается при сборке
`BuiltinRegistry`, dylib-плагины сканируются на старте, а `ToolRegistry`
собирается один раз для runtime/session.

Hot-swap здесь означает не "выгрузить dylib из процесса", а атомарно
переключить новые turn/model-request на новый snapshot модулей.

```text
AppConfig + Plugin scan + MCP discovery -> RuntimeSnapshot(epoch=N)
reload config/plugins/tools              -> RuntimeSnapshot(epoch=N+1)

running turn keeps epoch=N
new turn/model request uses epoch=N+1
```

## Инварианты

- Уже начатый `Workflow`, `Tool`, `ApprovalPolicy` decision или `PatchApplier`
  не меняет реализацию посреди вызова.
- Новые snapshots публикуются атомарно; старые trait-object `Arc` доживают до
  конца активных turns.
- Native dylib-плагины не выгружаются из процесса. Новая версия загружается как
  новый artifact/path/hash, старая остаётся в памяти до завершения процесса.
- Tool execution после reload всё равно идёт через `ToolRegistry`,
  `ToolOrchestrator`, `ToolSafety`, `ApprovalPolicy` и approval transport.
- Model-visible tool set может меняться чаще, чем сам registry, но только через
  `ToolExposure` или workflow host API.
- Каждый turn/event должен быть привязан к module epoch, чтобы UI/eval/debug
  могли восстановить, каким набором модулей был выполнен turn.

## Границы Перезагрузки

| Область | Граница reload | Комментарий |
|---|---|---|
| `Renderer` | per response / new request | Низкий риск, не влияет на permission. |
| `ToolExposure` | per model request | Подходит для deferred tool catalog и dynamic tool visibility. |
| `SearchBackend` | between turns или before context build | Нельзя менять во время одного context build. |
| `ContextBuilder` | between turns или before model request | Snapshot должен быть единым для выбранного context bundle. |
| `ToolRegistry` / tools | between turns, позже per model request | Исполнение всегда через `ToolOrchestrator`. |
| MCP-discovered tools | after explicit reload/discovery | Discovery обновляет новый snapshot, не мутирует старый registry. |
| `ApprovalPolicy` | only between turns | Иначе можно получить visibility по одной policy, execution по другой. |
| `PatchApplier` | only between turns | Edit semantics и approval context должны быть стабильны. |
| `Workflow` | only new turn | Текущий workflow frame не заменяется. |
| `MemoryStore` | only with durable-state rule | Нужен flush/migration или shared durable backend. |
| `ModelAdapter` | between turns/model requests | Требует явного event/debug, потому что меняет поведение агента. |

## Dynamic MCP Flow

Планируемый UX для запроса вроде "подключи github MCP":

1. Агент находит команду запуска MCP server и необходимые args/env.
2. Если нужно установить пакет или скачать binary, агент запрашивает approval.
3. Агент добавляет `[[tools.mcp_servers]]` в config.
4. Core получает явную команду `reload_tools` / `reload_modules`.
5. Новый snapshot выполняет MCP `initialize` + `tools/list`.
6. Discovered tools регистрируются в новом `ToolRegistry` с source
   `mcp:<server>` и safety floor не ниже `RunsCommands`.
7. Следующий model request видит новые tools напрямую или через deferred
   `ToolExposure`.

Важно: MCP server не получает обходной канал исполнения. Один host tool
мапится на один remote MCP tool, а model args не могут переопределить remote
tool name, command или server.

## Deferred Tool Exposure

Deferred/dynamic tool call не требует отдельного slot-а. Это реализация поверх
существующего `ToolExposure`:

```text
policy-visible ToolSpec candidates
    -> ToolExposure
    -> direct visible tools OR bridge tools
    -> model calls bridge tool
    -> bridge unwraps to real ToolCall.name
    -> ToolOrchestrator executes real tool
```

Bridge-tools вроде `tool_search`, `tool_describe` и `tool_call` должны быть
host-owned tools текущего snapshot. Их каталог строится из policy-visible tools
этого же snapshot и не должен видеть tools, не доступные session/profile.

При вызове `tool_call` events/debug могут показывать bridge invocation, но
approval, timeout, safety и result metadata должны относиться к реальному tool
name. Это сохраняет главный инвариант безопасности: dynamic discovery меняет
только видимость и token cost, а не путь исполнения.

## Минимальный Implementation Path

1. Добавить `ModuleEpoch`/`RuntimeSnapshot` как host-side concept без нового
   public slot.
2. Перенести `BuiltinRegistry` внутрь атомарно заменяемого snapshot holder для
   app-server/runtime.
3. Добавить explicit reload command в app-server protocol: сначала
   `reload_tools`, затем общий `reload_modules`.
4. При reload строить новый catalog/registry с нуля; старый snapshot не
   мутировать.
5. Испускать event вида `ModulesReloaded { old_epoch, new_epoch,
   changed_modules }`.
6. Добавить focused tests:
   - running turn продолжает использовать старый snapshot;
   - new turn видит новый tool/MCP config;
   - duplicate/failed plugin reload не ломает активный snapshot;
   - `tool_call` bridge не обходит `ApprovalPolicy`.

## Что Не Делать В v0

- Не выгружать native dylib из процесса.
- Не мутировать `ToolRegistry` активного turn-а in place.
- Не давать model-visible bridge tool вызывать произвольный command/tool вне
  текущего snapshot.
- Не добавлять feature-specific slots вроде `mcp_hot_reload` или
  `codex_tool_search`; сначала использовать `ToolExposure`, `Workflow`,
  `ToolRegistry` и app-server protocol.
