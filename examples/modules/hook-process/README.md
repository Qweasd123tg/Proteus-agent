# Перенос Скриптовых Хуков

Внешний JS/TS component для текущего `hook/v2`, без npm dependencies.
Нужен Node.js **22.18+**: `.mjs` работает как JavaScript, `.ts` — через
встроенный type stripping. TypeScript syntax с необходимой генерацией кода
(например, `enum`) требует предварительной сборки в JS. Внешние dependencies
устанавливаются владельцем рядом с entry; worker не устанавливает пакеты.

SDK уменьшает перенос до регистрации обработчика, сопоставления tool names/
arguments и разрешённого результата. Это **адаптер переноса**, а не режим
совместимости с полным Pi/OpenCode/Codex/Claude runtime. Upstream plugins
целиком не загружаются; пользователь явно выбирает переносимые handlers.

## Подключение

Добавьте в существующий profile следующие таблицы. Пути `args` и `entry`
ниже относительны рабочему каталогу component, то есть проекту; для работы
в другом проекте укажите абсолютные пути.

```toml
[modules]
hooks = ["ported-pi", "ported-opencode"]

[components.script-hooks]
command = "node"
args = ["examples/modules/hook-process/worker.mjs"]

[components.script-hooks.exports.hook.ported-pi]
timeout_ms = 5000

[components.script-hooks.exports.hook.ported-opencode]
timeout_ms = 5000

[module_config.hook.ported-pi]
entry = "examples/modules/hook-process/entries/pi.ts"
settings = {}

[module_config.hook.ported-opencode]
entry = "examples/modules/hook-process/entries/opencode.mjs"
settings = {}
```

`entry` обязателен, `settings` — необязательный object для owner config.
Неизвестные поля отвергаются. Module id свободный, exports одного component
разделяют process lifecycle/failure domain. Каждому export создаётся отдельный
registry handlers и передаётся только его `settings`. Module-level JS state
общий для повторных imports одного файла: state отдельного export держите
внутри `setup`, а не в глобальных переменных entry.

## Обычный Handler

```js
export default function setup(hooks) {
  hooks.on("before_tool", async (event, ctx) => {
    if (event.call.args.path?.endsWith(".env")) {
      return { action: "block_tool", reason: "Этот файл исключён из работы." };
    }
  }, { tools: ["read_file"] });
}
```

`setup` может быть async. `hooks.on(event, handler, {tools})` использует шесть
canonical событий. `tools` — непустой список точных имён только для
`before_tool`/`after_tool`; фильтр применяется до handler. Регистрация
заканчивается после setup; внутри export действует порядок регистрации,
между exports — `modules.hooks`. `undefined` означает `continue`.

Event/config/attribution доступны только для чтения. `ctx` содержит `cwd`,
`attribution`, `config` и invocation-owned `AbortSignal` (`signal`). Работу
следует отменять по этому signal; CPU loop блокирует Node event loop и
попадает под общий process timeout/reset. Worker принимает concurrent
invocations; shared state обработчиков синхронизирует сам автор.

`console.log` перенаправлен в stderr, чтобы обычный отладочный вывод
перенесённого кода не ломал protocol stdout. stdout зарезервирован за worker.
Host callbacks у этого slot отсутствуют; UI, model calls и tool registration
не появляются из-за использования SDK.

Types для редактора: [hooks.d.mts](hooks.d.mts), [ports.d.mts](ports.d.mts).
Canonical DTO и окончательная validation принадлежат Rust contract/host;
SDK не создаёт ещё одну копию полной model schema.

## Pi И OpenCode

Обёртки из [ports.mjs](ports.mjs) дают знакомые payloads для узких операций:

| Исходный handler | Регистрация | Поддержанная операция |
|---|---|---|
| Pi `tool_call` | `hooks.on("before_tool", piToolCall(handler))` | Изменение `input` или `block: true` с причиной |
| Pi `tool_result` | `hooks.on("after_tool", piToolResult(handler))` | Замена text content → Proteus `output` |
| OpenCode `tool.execute.before` | `hooks.on("before_tool", openCodeToolBefore(handler))` | Изменение `output.args` |
| OpenCode `tool.execute.after` | `hooks.on("after_tool", openCodeToolAfter(handler))` | Изменение `output.output` |

Примеры: [Pi/TS](entries/pi.ts), [OpenCode/JS](entries/opencode.mjs).
Для Pi context дополнительно содержит `hasUI: false`; UI API не эмулируется.
Имена и argument schema остаются Proteus: например, `read_file` и `args.path`
могут требовать замены исходного `read`/`filePath` в handler. Session id
доступен только при agent attribution; detached invocation получает `null`.

`piToolResult` применим только к unstructured результату. `isError`/`details`
можно читать, но менять status/metadata нельзя. Нет преобразования images
или structured data в текст с потерей информации. OpenCode `output.title`
не поддержан. Изменение status или неподдержанного поля — явная ошибка.
Handler exception проходит как обычная ошибка Proteus hook; для ожидаемого
veto используйте canonical `block_tool`, а не `throw`.

## Codex И Claude Code

Для JSON-stdin команд есть [runCommand](command.mjs) и
[обёртка PreToolUse](entries/command.mjs). Существующий shell/Python/JS script
можно сохранить, явно сопоставив его входные поля и решение.

```toml
[modules]
hooks = ["ported-command"]

[components.script-hooks]
command = "node"
args = ["examples/modules/hook-process/worker.mjs"]

[components.script-hooks.exports.hook.ported-command]
timeout_ms = 5000

[module_config.hook.ported-command]
entry = "examples/modules/hook-process/entries/command.mjs"

[module_config.hook.ported-command.settings]
command = "python3"
args = ["examples/modules/hook-process/entries/deny-edit.py"]
```

Обёртка передаёт `hook_event_name: "PreToolUse"`, `cwd`, `session_id`,
`turn_id`, `tool_name`, `tool_use_id`, `tool_input`. Tool names/arguments — Proteus;
для script, ожидающего другие имена, измените mapping в entry. Не создаются
fake `transcript_path`, `permission_mode` или `model`: такие зависимости
скрипта требуют отдельного переноса. `session_id` может быть `null`.

Поддержанное решение: JSON `hookSpecificOutput` с
`hookEventName: "PreToolUse"`, `permissionDecision: "deny"` и непустым
`permissionDecisionReason`; либо exit `2` с непустой причиной в stderr.
Exit `0` с пустым stdout/JSON `{}` продолжает обработку. `allow` также
продолжает обычный Proteus policy/approval path и не выдаёт разрешение.

`updatedInput` поддержан вместе с `permissionDecision: "allow"`: args
повторно проходят validation, policy и approval. Для Bash/apply_patch upstream
`command` нужно явно преобразовать в Proteus argument schema в entry.

Plain-text stdout, `additionalContext`, `systemMessage`,
`ask`, `continue: false` и другие неподдержанные поля не игнорируются: перенос
завершается явной ошибкой. Это отличие адаптера от оригинальных harnesses.
Exit codes кроме `0`/`2` тоже считаются ошибкой hook. Это помогает найти
участки, требующие адаптации, вместо молчаливой потери поведения.

`runCommand` запускает executable/argv без неявного shell, передаёт JSON
в stdin и ограничивает суммарный stdout/stderr 1 MiB. Для shell команды
укажите `command = "bash"`, `args = ["-c", "..."]` явно. На POSIX helper
владеет process group и завершает её при отмене/окончании script; на Windows
завершает только direct child. Timeout задаётся export-ом, а не отдельным
скрытым retry/timeout слоем helper.

Сверенные поверхности: [Pi extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md),
[OpenCode plugins](https://opencode.ai/docs/plugins/),
[Codex hooks](https://learn.chatgpt.com/docs/hooks#pretooluse),
[Claude Code hooks](https://code.claude.com/docs/en/hooks#pretooluse).

## Проверка Завершения

```js
export default function setup(hooks) {
  hooks.on("before_stop", (event) => {
    if (event.attempt === 0 && !event.output.text.includes("Проверено")) {
      return { action: "continue_turn", reason: "Проверь результат и укажи выполненные проверки." };
    }
  });
}
```

`before_stop` вызывается после успешного workflow перед финалом root turn.
`attempt = 0` — исходный кандидат. Первая причина завершает цепочку review;
host запускает тот же workflow с предыдущей историей, developer instruction
и `runtime.continuation`. Новое человеческое сообщение не создаётся. Общий
workflow timeout и cancellation действуют на все попытки; после восьми
продолжений очередной запрос завершает turn явной ошибкой. Кандидат сохранён
до ожидания review, поэтому отмена или ошибка reviewer сохраняет прогресс.
`TurnFinished` приходит клиенту только после принятия кандидата.

Для существующего Codex/Claude Stop script используйте [stop.mjs](entries/stop.mjs)
вместо `command.mjs` в том же config. В stdin поступают `hook_event_name: "Stop"`,
`cwd`, `session_id`, `turn_id`, `stop_hook_active`, `last_assistant_message`.
`stopDecision` переносит JSON `{ "decision": "block", "reason": "..." }`
или exit `2` с причиной в stderr в `continue_turn`. Пустой успешный ответ/`{}`
принимает кандидата. Это перенос решения: Codex Stop создаёт новый continuation
prompt; Proteus сохраняет root turn и передаёт причину как developer instruction.
`transcript_path`, system messages и остальные lifecycle API требуют адаптации.

## События И Границы Переноса

| Proteus | Близкая upstream задача | Граница |
|---|---|---|
| `turn_started` | Наблюдение начала обработки | Notification после принятия ввода; не `SessionStart` и не admission gate |
| `before_model` | Pi `context`, OpenCode context hooks | Canonical messages/instructions; схемы messages нужно сопоставлять явно |
| `before_tool` | Pi `tool_call`, OpenCode before, Codex/Claude `PreToolUse` | Veto или замена args; затем обычные validation/policy/approval |
| `after_tool` | Pi `tool_result`, OpenCode after, Codex/Claude post-tool обработка | Только текстовый `output`; фактический статус инструмента сохраняется |
| `before_stop` | Stop-проверка готового ответа | `continue_turn` с причиной, тот же root turn, максимум 8 продолжений |
| `turn_settled` | Наблюдение итогового результата | Notification; не actionable Stop, `agent_before_settle` или `session.idle` |

Session/fork/switch, отдельные compaction hooks,
provider/UI/command/tool registration требуют соответствующего Proteus
contract. Они не считаются успешно перенесёнными через приблизительный alias.

## Проверка

Из корня репозитория:

```bash
node --test examples/modules/hook-process/tests/*.test.mjs
./scripts/test.py -p proteus-core --test hook_runtime --test module_swap
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id js-hooks \
  --export '{"slot":"hook","module_id":"ported-pi","contract_version":"v2","module_config":{"entry":"examples/modules/hook-process/entries/pi.ts"}}' \
  --probe-export hook/ported-pi --probe-method hook.invoke \
  --probe-params '{"cwd":"/tmp","attribution":{"execution_id":"00000000-0000-0000-0000-000000000001","agent":null},"event":{"event":"before_tool","call":{"id":"00000000-0000-0000-0000-000000000002","name":"apply_patch","args":{},"surface":"function","raw_arguments":null},"spec":null,"blocked":null}}' \
  -- node examples/modules/hook-process/worker.mjs
```

Node checks проверяют перенос handlers, явные ошибки, TS loading,
multiplexing и targeted cancellation. Rust integration использует настоящий
component/slot, tool effect, canonical journal и workflow replay.
