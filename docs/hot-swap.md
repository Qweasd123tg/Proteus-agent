# Hot-Swap И Runtime Snapshots

Текущая реализация поддерживает snapshot-based `reload_tools`, а не полный
live reload process modules.

```text
AppConfig + Process descriptors + MCP discovery
  -> RuntimeSnapshot(epoch=N)

reload tools config
  -> RuntimeSnapshot(epoch=N+1)

running turn keeps N
next turn sees N+1
```

## Инварианты

- Turn захватывает один snapshot на старте.
- Workflow, policy, registry и adapters не меняются внутри turn.
- Новый snapshot строится полностью до публикации.
- Failed build не повреждает активный snapshot.
- Старые `Arc` и process sessions живут до завершения использующих их turns.
- Tool execution после reload остаётся в общем policy/safety path.
- `module_epoch` попадает в observability.

## Что Reload-ится Сейчас

`StdioRequest::ReloadTools` и `POST /reload-tools`:

1. перечитывают `tools.*` из config path;
2. заново строят catalog/registry snapshot;
3. выполняют MCP discovery;
4. публикуют новый epoch;
5. испускают `ModulesReloaded { old_epoch, new_epoch, tool_names }`.

`modules.*`, `process_modules`, provider и opaque module config этим
endpoint не переключаются. Для них app-server restart остаётся честной
границей.

## Process Lifecycle

Process module session принадлежит snapshot adapter-у. Когда старый snapshot
больше никем не удерживается, worker завершается вместе с adapter/host
lifecycle. Никакой native library в address space нет.

Смерть worker-а во время invocation даёт ошибку текущему вызову. Следующая
invocation той же session abstraction может lazily spawn child и повторить
handshake. Это recovery, не config hot-swap и не retry текущей операции.

## Dynamic MCP

Текущий flow:

1. config добавляет `[[tools.mcp_servers]]`;
2. пользователь явно вызывает reload;
3. новый snapshot выполняет MCP initialize + `tools/list`;
4. tools регистрируются с source `mcp:<server>` и safety floor
   `RunsCommands`;
5. следующий turn видит их через policy и tool exposure.

Model args не могут изменить downstream server/remote tool identity.

## Deferred Tool Exposure

Tool exposure можно вычислять per model request без замены registry:

```text
registered tools
  -> policy visibility
  -> ToolExposure
  -> direct tools / workflow bridge tools
  -> ToolRegistry execution
```

Bridge меняет model-visible catalog, но не execution authority.

## Полный Module Reload

Если он понадобится, минимальные требования:

- атомарно перечитать selection + descriptors + config;
- handshake всех новых selected workers до публикации;
- сохранить старый snapshot для активных turns;
- завершить новые workers при failed build;
- показать changed identities и failures;
- не переносить invocation state между module ids;
- проверить policy/tool surface consistency.

До реализации не называйте `reload_tools` полным module hot reload.

## Не Делать

- не мутировать registry активного turn;
- не переключать implementation между visibility и execution;
- не retry-ить failed invocation на другом module;
- не добавлять `module_id`-specific reload exceptions;
- не создавать отдельный native loader ради hot reload;
- не давать bridge tool обходить `ToolRegistry`.
