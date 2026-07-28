# Доверие, Tools И Границы Выполнения

Этот документ описывает фактическую модель выполнения Proteus. Несмотря на
историческое имя файла, отдельного policy/approval слоя больше нет.

## Коротко

Proteus рассчитан на локальное доверенное окружение:

- зарегистрированный и включённый tool доступен модели и исполняется напрямую;
- permission modes, approval requests, grants и approval cache отсутствуют;
- dylib, MCP server и process extension считаются доверенным кодом;
- shell по умолчанию запускается с обычными правами процесса Proteus;
- при необходимости оператор включает один process-level workspace sandbox
  через `PROTEUS_SHELL_SANDBOX=1`.

Это осознанное упрощение, а не обещание sandboxed plugin platform. Основная
граница runtime теперь отвечает за корректность orchestration и восстановимость,
а не за авторизацию каждого решения модели.

## Обязательный Путь Tool Call

Model-callable actions проходят один путь:

```text
ToolRegistry
  -> ToolExposure
  -> ToolOrchestrator
  -> schema validation
  -> journal record
  -> Tool::invoke
  -> timeout / cancellation / output bound
  -> canonical result
```

Runtime сохраняет следующие инварианты:

- неизвестное имя tool отклоняется до вызова;
- аргументы проверяются по JSON Schema;
- выключенный или не зарегистрированный tool не исполняется;
- timeout и cancellation проходят через общий orchestrator;
- слишком большой результат ограничивается до передачи модели;
- request, resolution и result связываются по `call_id` и пишутся в canonical
  journal;
- session/thread ownership и attribution проверяются на runtime-границах.

`ToolSafety` остаётся описательной классификацией для UI, диагностики и
конфигурации внешних tools. Она не является списком прав и сама по себе ничего
не разрешает и не запрещает.

## Что Считается Доверенным

### Dylib

Dylib-плагин исполняется внутри процесса core с теми же правами. Ошибка памяти,
panic через неподдержанную ABI-границу или вредоносный код могут повредить или
завершить процесс. Загружайте только плагины, которым доверяете, и пересобирайте
default pack вместе с совместимым `proteus-contracts`.

### MCP И Process Extensions

MCP stdio server, configured process tool, process SearchBackend и process
HistoryCompactor являются отдельными процессами, но не универсальной
песочницей. Host обеспечивает framing, request/response ids, timeout, bounded
frames, cancellation, restart и явную передачу environment. Доступ процесса к
filesystem и network определяется обычными правами ОС и способом его запуска.

`env_allowlist` копирует только явно названные переменные родителя, а `env`
задаёт scoped literals. Не храните secrets в tracked config и не считайте
очистку environment заменой sandbox.

### Provider-hosted Tools

`web_search` и `file_search` выполняются внутри provider request. Их side
effects и сетевой доступ находятся за границей локального `ToolOrchestrator`.
Включение hosted tool означает доверие выбранному provider и его реализации.

## Shell И Exec

`shell` и `exec_command` доверенные по умолчанию:

- команда запускается с текущими uid/gid, cwd, filesystem и network access;
- model-driven escalation и поля `with_escalated_permissions` /
  `justification` отсутствуют;
- внешний терминал, если поддерживается tool-ом, также не создаёт изоляцию.

Для process-level ограничения всего shell path запустите Proteus так:

```bash
PROTEUS_SHELL_SANDBOX=1 proteus
```

В этом режиме `shell-tool` использует `bwrap` и ограничивает запись workspace.
Запрос внешнего `workdir` или external terminal завершается ошибкой; отдельного
unsandboxed fallback и approval-пути нет. Если `bwrap` недоступен или не
запускается, sandboxed вызов завершается ошибкой до выполнения команды.

Sandbox является настройкой процесса, а не аргументом model tool call. Поэтому
он одинаков для всех shell-вызовов данного процесса и не может быть ослаблен
моделью посреди turn-а.

## Workspace Paths

File tools и `PatchApplier` проверяют workspace boundary, canonical parent и
недопустимые path escapes до операции. Абсолютный путь вне workspace, `..` и
symlink escape должны завершаться явной ошибкой.

Эти проверки относятся к конкретным стандартным tools. Произвольный dylib,
MCP/process tool или shell-команда может открыть путь самостоятельно и считается
доверенным кодом. Для реальной изоляции shell используйте
`PROTEUS_SHELL_SANDBOX=1`, а для сторонних процессов — отдельный OS/container
boundary.

## HTTP И Session Boundary

Loopback app-server предназначен для локального клиента. Wrapper создаёт
ephemeral session token и передаёт его web/Inspector. Non-loopback bind без
непустого token отклоняется до запуска runtime; CORS allowlist не заменяет
аутентификацию.

Token защищает transport endpoint, но не превращает app-server в публичный
multi-tenant service. Sessions сохраняют workspace/thread ownership; чужой
thread id, forged call id или повторная доставка terminal request должны
отклоняться.

## Journal, Timeout И Recovery

До потенциального side effect runtime записывает canonical tool request и
resolution `Allowed`; после выполнения — canonical result или terminal error.
Это позволяет видеть, что модель запросила и что вернулось в workflow.

Timeout/cancel не гарантирует откат внешнего side effect. Tool обязан
кооперативно завершать дочерние процессы, а recovery использует journal как
источник истины. Replay не исполняет реальные tools повторно и не должен
дублировать side effects.

## Чего Proteus Не Гарантирует

Proteus v0 не защищает от:

- вредоносного или скомпрометированного установленного extension;
- команды, которую модель запустила через trusted unsandboxed shell;
- provider-hosted side effect;
- filesystem/network доступа произвольного process tool;
- утечки секрета, явно переданного extension или provider;
- OS-level exploits и ошибок ядра/container runtime.

Практическое правило простое: включайте только нужные tools, проверяйте
источник extensions, не передавайте лишние secrets и используйте отдельный
workspace/container для недоверенных задач.

## Проверки

Минимальный статический smoke:

```bash
proteus --config codex doctor
proteus --config codex tools list
```

Для sandbox path отдельно проверьте запуск с
`PROTEUS_SHELL_SANDBOX=1`, успешную команду внутри workspace и отказ для
external `workdir`. Полная regression-матрица находится в
[testing.md](testing.md).
