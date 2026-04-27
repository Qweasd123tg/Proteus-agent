# Security И Policy

Security v0 держится на трёх уровнях:

1. tools объявляют `ToolSafety`;
2. `ApprovalPolicy` принимает решение перед исполнением;
3. сами tools проверяют workspace/path ограничения.

## ToolSafety

Поддерживаемые классы:

- `ReadOnly`;
- `WritesFiles`;
- `RunsCommands`;
- `Network`;
- `Dangerous`.

`ToolSpec` обязан описывать safety class. Policy не должна гадать по имени tool, если можно использовать `ToolSafety`.

## Встроенные Tools

| Tool | Safety | Поведение |
|---|---|---|
| `read_file` | `ReadOnly` | читает UTF-8 файл внутри workspace |
| `write_file` | `WritesFiles` | пишет UTF-8 файл внутри workspace |
| `shell` | `RunsCommands` | запускает команду в `cwd` |
| `search` | `ReadOnly` | вызывает выбранный `SearchBackend` |

## Workspace Boundary

`read_file` canonicalize-ит `cwd` и target path, затем проверяет, что файл находится внутри workspace.

`write_file` запрещает absolute path и parent traversal. Перед записью tool проверяет canonical workspace boundary для существующего target или parent directory, поэтому symlink не должен позволять запись за пределы workspace.

`shell` запускает команду с текущим `cwd`. В v0 дополнительной sandbox-изоляции внутри самого инструмента нет.

## ask_write

`ask_write` принимает решение в таком порядке:

1. если tool name в `allow`, разрешить;
2. если tool name в `ask_before`, запросить approval;
3. если `ToolSafety::ReadOnly`, разрешить;
4. если `ToolSafety::Dangerous`, запретить;
5. если `WritesFiles`, `RunsCommands` или `Network`, запросить approval;
6. если tool неизвестен, запретить.

Пример:

```json
{
  "policy": {
    "ask_write": {
      "ask_before": ["write_file", "shell"],
      "allow": ["read_file", "search"]
    }
  }
}
```

Важно: текущий workflow не имеет интерактивного approval transport. Если policy возвращает `Ask`, workflow пишет `ApprovalRequested`, затем `ApprovalResolved { approved: false }`, и возвращает tool result с ошибкой.

## allow_all

`allow_all` разрешает все tool calls. Используйте его только для тестов или доверенного окружения.

## Правила Для Новых Tools

- Всегда задавать корректный `ToolSafety`.
- Валидировать входной JSON до выполнения действия.
- Для file tools проверять workspace boundary.
- Для команд и сети считать действие потенциально опасным.
- Добавлять тест на policy behavior, если tool пишет файлы, запускает команды или ходит в сеть.
- Не исполнять tool в обход `ToolRegistry` и `ApprovalPolicy`.
