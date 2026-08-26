# План Сборки Runtime

`AssemblyPlan` — это единый развёрнутый чертёж между пользовательским config
и запуском runtime:

```text
AppConfig + ModuleCatalog
          |
          v
     AssemblyPlan       без запуска component workers
          |
          v
   contract validation
          |
          v
    RuntimeRegistry
          |
          v
     AgentRuntime
```

Он не является новым module slot, plugin API или wire protocol. Инвариант
`Core -> Contract -> Module Implementation` не меняется: план только один раз
фиксирует, что именно Core собирается подключить к существующим contracts.

## Зачем Он Нужен

Раньше runtime, `doctor`, topology и config reload независимо выводили выбор
modules и components из `AppConfig`. Даже одинаковая логика в нескольких
местах могла со временем разойтись.

Теперь план содержит одну точную картину:

- выбранный model profile и adapter;
- все 11 behavior slots и точный `module_id` каждого выбора;
- source реализации и `component_id` для process export-а;
- configured components, exports, contract versions и host callbacks;
- запрошенные tools, configured tools, MCP servers и subagent surface;
- предупреждения и блокирующие ошибки preflight-проверок.

Неизвестный выбранный module или повтор имени в `tools.enabled` блокирует план
до создания `RuntimeRegistry` и до подключения worker-а.

## Безопасная Сборка И Reload

`PreparedAssembly` объединяет уже проверенный `AssemblyPlan` и созданный из
него `RuntimeRegistry`. Runtime принимает их только вместе. При reload новая
пара полностью строится в стороне, затем одним обновлением заменяет текущий
`RuntimeSnapshot` и увеличивает `ModuleEpoch`.

Уже начатый turn продолжает использовать старый snapshot. Следующий turn
получает одновременно новый план и соответствующий ему registry; состояния
«план уже новый, а modules ещё старые» нет.

## Просмотр

Человекочитаемый вариант:

```bash
proteus --config codex inspect plan
```

Точная JSON projection:

```bash
proteus --config codex inspect plan --format json
```

У запущенного app-server текущий план доступен через:

```text
GET /inspect/plan
```

`inspect plan` не запускает component workers и не выполняет handshake. Поэтому
он показывает запрошенный tool surface, а фактически зарегистрированные tool
specs после сборки по-прежнему показывает `inspect topology`.

## Что Намеренно Не Попадает В JSON

JSON плана — read-only diagnostic projection, а не второй config format. В
него не сериализуются raw `AppConfig`, `module_config`, provider secrets,
component args или environment values. Загрузить JSON плана обратно и обойти
обычный config/contract validation path нельзя.

## Границы Гарантии

- План фиксирует protocol-visible authority по slot contract, а не по
  `module_id`.
- План не превращает process boundary в OS sandbox: workers остаются
  доверенными executable с правами пользователя.
- Точный model-visible список tools может меняться внутри invocation из-за
  policy и `ToolExposure`; статический план не подменяет эту проверку.
- План фиксирует выбранный process subagent runner и
  model-facing surface, но пока не сериализует безопасное summary ролей из
  opaque `module_config`. Конкретные запущенные экземпляры Proteus в план не
  входят: live agent tree является session-owned runtime state; см.
  [subagents.md](subagents.md).
- Runtime model/reasoning/permission overrides не переписывают неизменяемый
  план текущего module epoch; live значения отдельно отдаёт `/config` и
  topology.
- Это не общий hot reload. Сейчас атомарный путь используется при начальной
  сборке, Config Builder и поддержанном reload tools.

Следующий пользовательский слой поверх этой основы — сравнение двух планов
перед сохранением config-а. Оно должно быть projection над `AssemblyPlan`, а
не ещё одним способом собирать runtime.
