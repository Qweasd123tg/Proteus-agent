# Hot-Swap И Runtime Snapshots

Текущая реализация поддерживает snapshot-based `reload_tools`, а не полный
live reload process components.

```text
AppConfig + Process components/exports + MCP discovery
  -> PreparedAssembly(AssemblyPlan + RuntimeRegistry)
  -> RuntimeSnapshot(epoch=N)

reload tools config
  -> PreparedAssembly(AssemblyPlan + RuntimeRegistry)
  -> RuntimeSnapshot(epoch=N+1)

running turn keeps N
next turn sees N+1
```

`RuntimeSnapshot` фиксирует assembly/configuration view, а не continuation
вычисления. Он не содержит program counter, stack, local state Workflow или
suspended future и не позволяет продолжить оборванный Turn после crash.

## Инварианты

- Turn захватывает один snapshot на старте.
- Workflow, policy, registry и adapters не меняются внутри turn.
- План и соответствующий registry всегда публикуются одной парой.
- Новый snapshot строится полностью до публикации.
- Failed build не повреждает активный snapshot.
- Старые `Arc` и process sessions живут до завершения использующих их turns.
- Tool execution после reload остаётся в общем policy/safety path.
- `module_epoch` попадает в observability.

Завершённая `ExecutionScope` migration сохранила этот инвариант: один
`ExecutionContext` собирается из одного captured `RuntimeSnapshot`, а generic
context не делает новый lookup из mutable published registry на каждом step.
История Phase 2 и её evidence сохранена в
[архивном roadmap](../archive/roadmap-through-2026-08-31.md#executionscope-migration).

## Что Reload-ится Сейчас

`StdioRequest::ReloadTools` и `POST /reload-tools`:

1. перечитывают `tools.*` из config path;
2. заново строят и проверяют `AssemblyPlan`;
3. из него собирают catalog/registry snapshot;
4. выполняют MCP discovery;
5. публикуют новый epoch;
6. испускают `ModulesReloaded { old_epoch, new_epoch, tool_names }`.

`modules.*`, `components`, provider и opaque module config именно этим
endpoint не переключаются. Config Builder отдельно умеет атомарно применить
поддержанные selection/provider/module-config поля через тот же
`PreparedAssembly`; он не создаёт новые component definitions. Для ручного
изменения launch topology app-server restart остаётся честной границей.

## Process Lifecycle

Process component session разделяется его export adapters внутри snapshot-а.
Когда старый snapshot больше никем не удерживается, worker завершается вместе
с launcher/host lifecycle. Никакой native library в address space нет.

Смерть worker-а во время invocation даёт ошибку текущему вызову. Следующая
invocation любого export той же session abstraction может lazily spawn child
и повторить exact-set handshake. Это recovery, не config hot-swap и не retry
текущей операции.

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

- атомарно перечитать selection + components/exports + config;
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
