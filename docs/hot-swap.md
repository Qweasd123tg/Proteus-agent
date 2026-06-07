# Hot-Swap И Dynamic Modules

Этот документ фиксирует boundary горячей замены модулей. Текущая реализация
поддерживает snapshot-based reload для app-server tools/config/MCP discovery.
Полный `reload_modules`, persistent MCP host и dylib unload остаются planned.

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

Bridge-tools называются `proteus_tool_search`, `proteus_tool_describe` и
`proteus_tool_call` и принадлежат `coding-workflow`, а не `ToolRegistry`. Их
каталог строится из policy-visible tools текущего snapshot через
`visible_tools_json` и не видит tools, не доступные session/profile.

При вызове `proteus_tool_call` workflow создаёт внутренний `ToolCall` с
реальным tool name и передаёт его в `execute_tool_json`. Transcript result
получает outer call id, чтобы provider видел ответ на свой вызов
`proteus_tool_call`, а inner call id сохраняется в metadata. Это сохраняет
главный инвариант безопасности: dynamic discovery меняет только видимость и
token cost, а не путь исполнения.

## Минимальный Implementation Path

Сделано:

1. `ModuleEpoch`/`RuntimeSnapshot` добавлены как host-side concept без нового
   public slot.
2. `BuiltinRegistry` живёт внутри snapshot holder; `AgentRuntime::run` берёт
   snapshot один раз на старте turn-а.
3. App-server protocol поддерживает `StdioRequest::ReloadTools`; HTTP даёт
   `POST /reload-tools`. Эта команда применяет только `tools.*` из config path;
   `modules.*` остаются задачей будущего `reload_modules`.
4. Reload строит новый catalog/registry с нуля и публикует новый snapshot, не
   мутируя старый.
5. App-server испускает `AppServerEvent::ModulesReloaded { old_epoch,
   new_epoch, tool_names }`.
6. Тесты покрывают, что running turn продолжает использовать старый snapshot,
   а new turn видит новый tool config.

Осталось:

- добавить общий `reload_modules`, если понадобится reload не только
  config-defined/MCP/plugin tool graph;
- расширить reload report до `changed_modules`, а не только `tool_names`;
- покрыть failed plugin reload без повреждения активного snapshot;
- реализовать `tool_call` bridge и проверить, что он не обходит
  `ApprovalPolicy`.

## Что Не Делать В v0

- Не выгружать native dylib из процесса.
- Не мутировать `ToolRegistry` активного turn-а in place.
- Не давать model-visible bridge tool вызывать произвольный command/tool вне
  текущего snapshot.
- Не добавлять feature-specific slots вроде `mcp_hot_reload` или
  `codex_tool_search`; сначала использовать `ToolExposure`, `Workflow`,
  `ToolRegistry` и app-server protocol.
