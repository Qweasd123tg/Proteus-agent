# Security И Policy

Security path зарегистрированных tools в v0 держится на четырёх уровнях:

1. tools объявляют `ToolSafety`;
2. `PermissionMode` оборачивает configured `ApprovalPolicy` в mode-aware policy;
3. `ToolOrchestrator` спрашивает `ApprovalPolicy` отдельно для visibility и execution;
4. сами tools проверяют workspace/path ограничения.

Facade-tool `task` проходит тот же путь
`ToolRegistry -> mode-aware ApprovalPolicy -> ToolOrchestrator -> Tool::invoke`;
worktree для пишущей роли создаётся только после разрешения. Остальные current
gaps перечислены в разделе «Известные Ограничения Текущей Реализации».

Этот документ описывает текущую реализацию v0. Более гибкая config-editable
модель прав остаётся planned и кратко описана в конце.

Для alpha reporting, поддерживаемой линии и короткой trust-boundary сводки
используйте корневой [SECURITY.md](../SECURITY.md). Process component считается
доверенным executable: strict protocol ограничивает его callbacks в host, но
не ограничивает прямые OS-действия самого процесса.

В v0 нет универсального OS sandbox для всех tools. Текущая защита держится на
workspace boundary, safety classes, permission mode и approval policy; для
process tool module `shell-tool` дополнительно использует
bwrap-песочницу (см. «Exec Sandbox В shell-tool» ниже). Общий network gate,
protected paths и secrets policy являются следующими слоями, а не заменой
текущего `ToolOrchestrator`.

## Доверенные Process Components

Каждый `[components.<id>]`, у которого snapshot строит используемый export,
запускает настроенный локальный executable. Его exports реализуют module slots, но component сам
не является model-callable tool: `ToolSafety`, approval policy и shell sandbox
не оборачивают child process. Worker работает с правами самого Proteus,
поэтому в config нельзя подключать недоверенную команду. Выбор делается
реальным `module_id`, а не служебным id `process`.

Несколько exports одного component делят те же ambient OS-права и process
state. Protocol-visible authority при этом не объединяется: host разрешает
callback только по активному export. Это защищает control plane от случайного
или ошибочного вызова, но не изолирует private code внутри доверенного binary.

При этом tool callback из process Workflow не получает исключения: методы
`host.tools.execute`/`host.tools.execute_batch` возвращаются в core и проходят
обычный `ToolRegistry -> mode-aware ApprovalPolicy -> ToolOrchestrator ->
Tool::invoke`. Worker не задаёт `ToolInvocationOwner` и не может выдать себе
turn grants; owner и cancellation берутся из текущего host invocation context.

Host очищает parent environment и передаёт только минимальные runtime variables,
явный `env_allowlist` и literal `env`. Строгий handshake защищает от ошибочно
подключённого executable и exact export set, но не является sandbox или
границей доверия.

## Rust LSP Process Boundary

`lsp_diagnostics` является model-callable tool, поэтому, в отличие от process
module, всегда проходит `ToolRegistry -> ApprovalPolicy -> ToolOrchestrator`.
Он помечен `RunsCommands`: `rust-analyzer` и запускаемые им Cargo/toolchain
компоненты не являются чистым чтением и после approval работают с правами
Proteus, без bwrap-песочницы `shell-tool`. Process environment очищается;
передаются `PATH` и ограниченный набор `HOME`/Cargo/Rustup переменных, нужный для
поиска toolchain. Путь документа отдельно ограничен существующим `.rs` внутри
workspace, включая canonical symlink check. Подменять отсутствующий
`rust-analyzer` неявным `cargo check` запрещено.

## App-Server HTTP Boundary

`proteus server http` предназначен для локального web-клиента и dogfood
запусков. Держите bind только на `127.0.0.1` и не экспонируйте порт в сеть:
HTTP endpoints умеют отправлять prompts, approvals, typed input, cancel,
reload-tools, history/resume, inspect topology diagnostics и shutdown.

У прямого запуска `proteus server http` token auth по умолчанию выключен только
для loopback bind; включить его можно через `--token <token>`. Любой
non-loopback bind требует непустой token и отклоняется до запуска runtime и
`bind`, если auth не включён. Установленный wrapper из `install.sh` строже: если
`PROTEUS_SESSION_TOKEN` не задан, он генерирует ephemeral token на каждый
запуск. Отключение wrapper token-mode только явное:
`PROTEUS_NO_SESSION_TOKEN=1`.

Когда token-mode включён, session token требуется для любого HTTP endpoint,
кроме preflight `OPTIONS` и `GET /health`; правило применяется централизованно
и не зависит от ручного списка routes. Для SSE допустим query token, потому что
browser `EventSource` не выставляет произвольные headers; для обычных `fetch`
requests предпочтителен `X-Proteus-Session` или
`Authorization: Bearer <token>`. Raw token не печатать в обычные logs и не
класть в `localStorage`; in-memory state или `sessionStorage` приемлемы для v0.

Direct CLI и HTTP server boundary fail-closed связывают non-loopback bind с
обязательным token: например, `--host 0.0.0.0` или `--host ::` без `--token`
завершаются ошибкой. CORS/`Origin` не заменяют auth; наличие token также не
превращает app-server в production-ready public service.

CORS для защищённых endpoints должен быть allowlist-ом локальных origins,
например chat `http://127.0.0.1:1420`, inspector
`http://127.0.0.1:1421`, соответствующие `localhost` origins и текущий
dev-server port. Wildcard CORS допустим только для явно публичных endpoints
вроде `/health`; requests без `Origin` от локальных CLI/curl можно принимать
при валидном token.

## ToolSafety

Поддерживаемые классы:

- `ReadOnly`;
- `WritesFiles`;
- `RunsCommands`;
- `Network`;
- `Dangerous`.

`ToolSpec` обязан описывать safety class. Policy не должна гадать по имени tool, если можно использовать `ToolSafety`.

## PermissionMode

`permissions.mode` задаёт режим доступа:

- `plan` показывает и исполняет только `ReadOnly` tools;
- `normal` использует `ApprovalPolicy` и `ApprovalTransport`;
- `auto` разрешает `ReadOnly` и `WritesFiles` без approval, но запрещает `RunsCommands`, `Network` и `Dangerous`.

Runtime применяет режим через `ModeAwarePolicy` на границе сборки
`RuntimeContext`. `ToolOrchestrator` не знает про конкретные режимы и
делегирует visibility/execution одному `ApprovalPolicy`. Композиция
deny-monotonic: явный `Deny` выбранной policy (или structural deny при её
отсутствии), а также deny-правило `codex_policy`/`opencode_policy` остаётся
`Deny` в любом режиме. `plan` и
`auto` могут снять только `Ask` для разрешённого режимом safety class; их
собственные запреты по `ToolSafety` по-прежнему являются верхней границей.

CLI может переопределить config через `--plan`, `--auto` или
`--permission-mode plan|normal|auto`. Внешние UI-клиенты могут переключать
режим для следующих turns через `StdioRequest::SetPermissionMode`.
Переключение не меняет config-файл и не перезапускает app-server. В client-side
plan flow UI может просить модель вернуть staged read-only plan, а после ответа
предлагать execute/revise/dismiss; enforcement read/write/shell/network
ограничений остаётся в core policy. Если workflow возвращает
`metadata.ui.plan_intake`, UI показывает generic form для уточняющих выборов;
эти ответы являются обычным следующим user turn и не дают обхода
`ModeAwarePolicy`.

## Встроенные Tools

| Tool | Safety | Поведение |
|---|---|---|
| `apply_patch` | `WritesFiles` | применяет workspace-scoped patch через `PatchApplier` |
| `remember_fact` | `WritesFiles` | кладёт preference/fact в `MemoryStore` (пишет в SQLite/JSONL, не в workspace-файлы) |
| `search` | `ReadOnly` | вызывает выбранный `SearchBackend` |
| `request_user_input` / `AskUserQuestion` | `ReadOnly` | запрашивает typed ответ через `UserInputTransport`; второй id — provider-compatible alias |
| `task` | `WritesFiles` | foreground subagent facade; может запустить writing/worktree роль и потому проходит write approval boundary |
| `spawn_agent` | `WritesFiles` | экспериментальный async subagent spawn; доступен только для `parallel_safe`, `isolation = none` ролей, но сохраняет консервативный safety floor |
| `send_message` / `followup_task` | `WritesFiles` | сообщение способно направить активный child tool loop, а follow-up — запустить resumable turn; оба сохраняют тот же консервативный approval boundary |
| `list_agents` / `wait_agent` / `interrupt_agent` | `ReadOnly` | session-owned collaboration control без прямой записи workspace; `interrupt_agent` меняет только lifecycle принадлежащего session ребёнка |

File I/O (`read_file`, `write_file`, `list_dir`, `grep`, `find_files`,
`read_many_files`), git helpers (`git_status`, `git_diff`) и `shell` вынесены
из ядра в process modules `file-tools`, `git-tools` и `shell-tool`
соответственно. Добавьте `tool/reference.tools` component export и нужные имена
в `tools.enabled`. Safety
каждого process tool декларируется в его `ToolSpec` и проверяется тем же
механизмом, что и core facade tools.

Process tool names валидируются при регистрации: пустое имя и duplicate между
modules отклоняются. Если явно включённый process tool совпал с
builtin/configured tool, сборка registry завершается ошибкой конфигурации;
приоритет или silent skip не применяются.

Subagent facade выбирается top-level полем `subagents.surface`. В режиме
`task` регистрируется только `task`; в `collaboration` — базовые
`spawn_agent`, `list_agents`, `wait_agent`, `interrupt_agent` и, для runner-а с
message capability, `send_message`/`followup_task`; `none` не
регистрирует ни одну поверхность. Это реальные registry tools, а не workflow
side-channel, поэтому mode-aware visibility, approval, timeout и result bounds
остаются обязательными.

Collaboration control скоупится по `SessionId`: path `/root/<task_name>` нельзя
адресовать из другой session. Запуск допускает только явно `parallel_safe`
роль без worktree isolation, а дочерний toolset лишён всех subagent facade
tools, поэтому nesting в первом slice невозможен. Records и terminal payloads
bounded, active records не вытесняются; состояние process-resident и теряется
при restart. Send/follow-up/fork и writer/worktree spawn в этом режиме не
реализованы, а несовместимый blocking-only subagent runner отклоняется при сборке
registry без fallback.

Config-defined `native` tools не могут понизить safety ниже safety встроенного handler-а. Например `native.handler = "apply_patch"` останется `WritesFiles`, даже если config укажет `ReadOnly`. File I/O и shell больше не доступны через `native.handler` — они приходят из process tool modules.

Config-defined `process`, inline stdio `mcp` и discovered
`tools.mcp_servers` tools также считаются command execution boundary. Даже
если config укажет `ReadOnly` или `WritesFiles`, runtime поднимает effective
safety до `RunsCommands`, поэтому такие tools не видны и не исполняются в
`plan` и запрещены в `auto`.

Process-based built-in tools читают stdout/stderr через bounded reader: модуль
сохраняет только первые bytes лимита и дочитывает остаток без накопления в
памяти. После этого `ToolOrchestrator` всё равно применяет общий output
truncation перед событием `ToolFinished` и передачей результата модели. Дефолтный
лимит orchestrator-а — `200_000` bytes; при обрезке в `output`/`error`
добавляется явный marker, а metadata получает `output_truncated` /
`error_truncated`, original byte count и `max_output_bytes`.

Для `mcp` один host tool всегда мапится на один фиксированный remote MCP tool
из config или результата `tools/list`. Model args не могут переопределить
remote tool name; это сохраняет связь между `ToolSpec`, policy decision и
фактическим downstream вызовом.

Persistent stdio host дополнительно ограничивает receive backlog: reader queue
и ещё не drained JSON-RPC notifications делят один budget по числу кадров и
суммарному compact-JSON размеру (по умолчанию 256 кадров / 32 MiB), а framing
отдельно ограничивает размер одного wire frame. При исчерпании budget reader
останавливается с явной ошибкой; он не продолжает накапливать валидные, но
невостребованные сообщения.

Process modules, inline/discovered MCP и configured executor `kind = "process"`
используют одну fail-closed environment policy из `ProcessSpec`. На Unix по
умолчанию наследуется только `PATH`; Windows дополнительно сохраняет
необходимые system/process/temp variables. Все эти config-пути принимают
`env_allowlist = ["TOKEN_NAME"]` для scoped копирования значения из parent и
`env = { NAME = "value" }` для literal child-only значений. Literal перекрывает
allowlisted parent value. `HOME`, cloud/API tokens, proxy variables и agent
sockets автоматически не передаются. Для credentials предпочтителен allowlist,
чтобы secret value не попадал в config-файл.

### Provider-hosted tools

`web_search` и `file_search` OpenAI Responses регистрируются как виртуальные
registry tools с source `provider_hosted:openai.responses`, surface
`ProviderHosted` и обязательным `ToolSafety::Network`. Они проходят те же
duplicate-name и visibility checks, но выполняются OpenAI внутри model request,
а не локальным `Tool::invoke`.

Из-за этой временной границы per-call approval после выбора модели невозможен:

- `PolicyDecision::Allow` добавляет hosted tool в model request;
- `Ask` и `Deny` скрывают его, даже если UI умеет показывать approvals;
- `plan`/`auto` скрывают его по network safety;
- случайный local/deferred вызов возвращает ошибку без side effect;
- provider response с function/custom call под именем hosted tool отклоняется
  protocol validator-ом: допустим только canonical `HostedToolActivity`.

Таким образом, policy pre-authorizes саму возможность provider-side доступа к
сети или vector store. Локальная shell sandbox и approval после получения
`web_search_call` эту операцию защитить уже не могут. Hosted tools не нужно
указывать в `tools.enabled`; operator включает их в provider config и отдельно
разрешает canonical имена в выбранной policy.

## Workspace Boundary

`apply_patch` остаётся core tool-ом, но сам алгоритм применения patch живёт в
выбранном `PatchApplier`. Reference module `direct-patch` канонизирует `cwd` и target
path перед записью и отклоняет absolute paths, parent traversal и
symlink-escape; конечный symlink запрещён для Add/Update/Delete и обеих сторон
Move, даже если он указывает обратно внутрь workspace. `ToolOrchestrator` не
делает workspace-санитизации за `PatchApplier` — это обязанность выбранной
реализации.
В packaged proxy-профилях `codex`/`glm` model-facing форма `apply_patch` —
обычный function tool. Явно настроенный freeform custom tool всё равно проходит
через тот же `ToolOrchestrator`, `ApprovalPolicy` и `PatchApplier`.

Tools из текущих reference modules `file-tools` (`read_file` / `write_file` / `list_dir` /
`grep` / `find_files` / `read_many_files`), `git-tools` (`git_status` /
`git_diff`) и `shell-tool` применяют свои
собственные проверки workspace-boundary. Core не гарантирует эту проверку за
implementations — это обязанность автора module.
Reference tool implementations должны использовать общие helper-ы
`proteus_contracts::tool_support::{workspace_path, workspace_path_for_write}`,
чтобы read/write path handling не расходился. После process-only cutover
одинаковый Tool invocation contract не отменяет implementation-local path
validation или host policy.
`write_file` может создавать недостающие parent directories, но только после
лексической проверки пути, запрета `..` и проверки symlink parents, чтобы
создание не уходило за пределы workspace.

## ask_write

`ask_write` экспортируется reference worker-ом из `policy-pack`; core применяет
его через обычный process `ApprovalPolicy` slot без исключений по id.

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
  "module_config": {
    "policy": {
      "ask_write": {
        "ask_before": ["apply_patch", "remember_fact"],
        "allow": ["search"]
      }
    }
  }
}
```

Важно: решение `ask_write` без ослабления применяется в
`permissions.mode = "normal"`. В `plan`/`auto` его `Deny` сохраняется, а `Ask`
может быть снят только для разрешённого режимом safety class. CLI single-run и
line REPL имеют интерактивный `ApprovalTransport`. Если итоговая policy
возвращает `Ask`, `ToolOrchestrator` пишет `ApprovalRequested`, ждёт ответ
transport, затем пишет `ApprovalResolved` и исполняет tool только при
`approved: true`.

Runtime оборачивает выбранный transport в session-level approval cache. Cache
используется только если approval response явно вернул cache scope:

- `none` — не кэшировать;
- `exact_call` — ключ `cwd + tool name + canonical JSON args`;
- `exact_command` — command-shaped exact call для shell/process UX, ключ тот же
  что у `exact_call`;
- `workspace_write` — широкий cache для workspace-scoped write tools, ключ
  `cwd + tool name`; принимается только если `ToolSpec.metadata.approval`
  явно содержит `workspace_write`;
Ключ кеша дополнительно включает `thread_id` запросившего из
`ApprovalRequest.origin`: approve, выданный одному исполняющему контексту
(main loop или конкретный запуск субагента), не переиспользуется другим.
Main thread стабилен на всю сессию, поэтому для основного цикла кеш работает
как раньше; субагентный child-thread живёт один запуск — его approvals
истекают вместе с ним. Запросы без origin образуют собственный bucket.

Core санитайзит слишком широкие scopes: shell/command/network/dangerous tools
не получают broad cache, а неподходящий `workspace_write` понижается до
`exact_call`. Неизвестный wire scope понижается до `none`, чтобы будущие клиенты
не ломали текущий server. Cache хранится только в памяти текущего
runtime/session и не переживает restart или `resume_from_session_dir`.
`ToolSpec.metadata.approval.cache.disabled = true` полностью выключает cache
для конкретного tool-а. `request_permissions` выставляет этот generic opt-out:
его approval не читается из cache и не записывается туда даже внутри одного
turn. Каждый вызов снова проходит через реальный transport, иначе cached
approval из прошлого turn мог бы без нового согласия выпустить новый
turn-scoped grant.

App-server approval request может содержать optional `preview` для UI. Это
подсказочные метаданные: они помогают клиенту показать, что будет одобрено, но не
меняет execution semantics. Safety boundary остаётся прежней:
`ToolRegistry` выбирает зарегистрированный tool, mode-aware `ApprovalPolicy`
принимает visibility/execution decision, `ToolSafety` задаёт нижний safety
floor, а сам tool выполняет validation и workspace/path checks перед действием.

Текущий WIP preview покрывает три частых approval сценария:

- `apply_patch` - affected files и patch/diff body из JSON `patch` или
  freeform `input`;
- `write_file` из `file-tools` - affected file и content/diff body;
- `shell` из `shell-tool` - command body, cwd и command-oriented metadata.

UI не должен использовать `preview` для обхода `ask_before`, cache sanitation,
workspace boundary или аргументной validation. Если `preview` отсутствует,
approval остаётся валидным и должен рендериться через обычные `ToolCall`,
`reason`, `cwd` и `tool_spec`.

Headless runtime без approval transport отказывает `Ask`. App-server transport
публикует `ApprovalRequested` и ждёт ответ UI-клиента через `approval`.
Pending request хранится в app-server и доступен через `GET /pending`, поэтому
краткий SSE reconnect не отклоняет approval сам по себе. Если клиент не
ответил до ненулевого `app_server.approval_timeout_ms`, app-server отклоняет
approval и очищает pending request. При дефолтном значении `0` timeout
отключён, и интерактивный prompt ждёт пользователя до ответа, cancel или
shutdown. При shutdown app-server отклоняет все pending approvals.

Очередь pending approvals атрибуцирована и per-request scoped:

- `ApprovalRequest.origin` несёт `RequestOrigin` — `thread_id`/`turn_id`
  исполняющего контекста и optional `label` — субагентный runner ставит туда
  имя роли через `RuntimeContext.thread_label`. На wire
  (`AppApprovalRequest.origin`) attribution опциональна: старые клиенты и
  серверы совместимы.
- `AppApprovalRequest.seq` — монотонный порядковый номер очереди; `GET
  /pending` и web-клиент сортируют pending approvals по нему, а не по
  случайному UUID.
- Каждый pending approval привязан watcher-ом к своему запросившему: если
  orchestrator дропает approval future (cancel turn-а, timeout субагента),
  запись удаляется и клиентам уходит `ApprovalResolved {approved: false}`.
  Поэтому cancel одного turn-а больше не отклоняет pending approvals других
  конкурентных turn-ов; blanket-deny остаётся только на shutdown.
- Терминальный transport CLI сериализует конкурентные prompts mutex-ом и
  печатает `from: subagent '<role>'` для запросов дочерних циклов; web-клиент
  показывает бейдж роли на approval-карточке.

Очередь pending user inputs (`request_user_input`) устроена зеркально:
orchestrator оборачивает `UserInputTransport` attribution-обёрткой, поэтому
`UserInputRequest.origin` несёт тот же `RequestOrigin`, forwarder app-server-а
присваивает `UserInputRequest.seq`, watcher убирает запись при смерти
запросившего (клиентам уходит `UserInputResolved`), а blanket-resolve пустыми
ответами остаётся только на shutdown. Оба поля serde-tolerant: старые
payload-ы без них парсятся с defaults.

`ToolOrchestrator` передаёт модели tools через
`ApprovalPolicy::evaluate_visibility`: tools с `Allow` видны сразу, tools с
`Ask` видны только если transport умеет интерактивно запросить approval, а
`Deny` tools не попадают в candidates для `ToolExposure`. После этого
`ToolExposure` может только сузить/ранжировать список перед
`CanonicalModelRequest.tools`, но не может вернуть запрещённый policy tool. При фактическом
вызове `ToolOrchestrator` использует `ApprovalPolicy::evaluate` с реальным
`ToolCall`, поэтому execution policy видит аргументы модели и не зависит от
fake visibility call.

Если `Tool::invoke` возвращает ошибку или превышает `ToolSpec.timeout_ms`, `ToolOrchestrator` не роняет turn целиком: он пишет `ToolFinished` с `ToolResult { ok: false }` и передаёт ошибку модели как tool result. Большой `output`/`error` обрезается единым лимитом orchestrator-а с visible truncation marker и metadata о truncation.

`ToolContext` содержит `CancellationToken`, чтобы long-running tools могли
кооперативно остановиться. Текущие built-in tools пока в основном полагаются на
host timeout/`kill_on_drop`, но contract уже не требует менять сигнатуру при
добавлении cooperative cancellation.

`ToolResult.output` остаётся text fallback для текущих adapters. Для platform
path добавлен `ToolResult.content: Vec<ToolContent>` с text/json/image/binary
blocks; новые tools могут возвращать structured output без изменения DTO.

`ToolSpec.surface` описывает только то, как tool показывается модели
конкретным provider adapter-ом (`function`, `freeform` и т.п.). Он не
понижает `ToolSafety`, не обходит visibility/execution policy и не меняет
executor. Если adapter не поддерживает surface, он должен вернуть ошибку
model request, а не делать эвристический fallback к другой форме. Для
freeform это выражено capability `supports_freeform_tools`; ответ provider-а с
другой surface отклоняется до history mutation и исполнения tool-а.

Core не валидирует внутреннюю схему `ask_write`: значение
`module_config.policy.ask_write` передаётся в `policy-pack` как JSON. Имена в
`allow`/`ask_before` влияют только на реально зарегистрированные tools.

## Exec Sandbox В shell-tool

Reference module `shell-tool` (tools `shell`, `exec_command`) сам заворачивает
неэскалированные команды в OS-песочницу `bwrap` (bubblewrap):

- `--unshare-net`: у каждой команды собственный network namespace, внешней сети
  нет. Важное следствие: localhost-сервер, поднятый одним sandboxed-вызовом,
  недостижим из любого другого вызова и с машины пользователя, хотя сам процесс
  стартует успешно. Серверы, к которым нужен доступ, должны запускаться с
  `with_escalated_permissions: true`. Это поведение задокументировано для модели
  в описаниях tools и в секции «Sandbox and escalation» prompt-профиля
  `codex-default.md`.
- `--unshare-pid` вместе с `--proc /proc`: команда видит procfs своего PID
  namespace и не может адресовать процессы host-а по их host PID. Это гарантия
  только этой process boundary, а не общая гарантия изоляции всех IPC-каналов.
- корень ФС монтируется read-only, только workspace — read-write, `/tmp` —
  свежий tmpfs, `/dev` и `/proc` — новые. Неэскалированный `workdir` обязан
  находиться внутри workspace; проверка canonical path учитывает `..` и
  symlink;
- `--die-with-parent`: команда умирает вместе с host-процессом.

Вызов с `with_escalated_permissions: true` исполняется без песочницы и проходит
через approval (см. `codex_policy` ниже). Если `bwrap` недоступен, не executable
через `PATH` или выставлен `PROTEUS_SHELL_SANDBOX=0`, неэскалированный вызов
завершается ошибкой до spawn; для явно запрошенного unsandboxed run нужна
эскалация. Путь внешнего терминала (Ptyxis) также считается unsandboxed,
допускается только для эскалированного вызова и сообщает `sandbox: null`,
`escalated: true`.

Независимо от песочницы все команды `shell`/`exec_command` получают
env-нейтрализацию интерактивности: `PAGER`/`GIT_PAGER`/`GH_PAGER=cat`,
`TERM=dumb`, `NO_COLOR=1`, `COLORTERM=""`, `LANG`/`LC_CTYPE`/`LC_ALL=C.UTF-8`
и маркер `PROTEUS_CI=1`. Это копия `UNIFIED_EXEC_ENV` из upstream Codex
(брендовый `CODEX_CI` заменён на `PROTEUS_CI`): без неё `git diff`/`gh`
повисают на интерактивном pager-е внутри PTY-сессии.

Upstream Codex использует другой механизм (seatbelt/landlock+seccomp), где
сервер в песочнице просто не может забиндиться — ошибка громкая. Наш
bwrap-путь допускает тихий старт сервера в изолированной сети, поэтому
per-command изоляция описана модели явно; это задокументированная divergence
поведения exec-песочницы, а не workflow/policy контрактов.

## codex_policy

`codex_policy` поставляется тем же `policy-pack` и используется
экспериментальным named config `codex` (`configs/codex.config.toml`). Это не отдельный
security layer: core применяет его через тот же `ApprovalPolicy` slot и тот же
mode-aware wrapper.

Порядок решения:

1. если tool name в `deny`, запретить — deny побеждает всё, включая
   `allow_sandboxed`;
2. если tool name в `allow_sandboxed`: не-эскалированный вызов разрешается без
   approval (tool обязан создать песочницу или завершиться ошибкой без запуска;
   unsandboxed fallback запрещён), а вызов с
   `with_escalated_permissions: true` требует approval — кроме случая, когда на
   этот ход уже выдан грант `escalated_exec` (см. ниже);
3. если tool name в `allow`, разрешить;
4. если tool name в `ask_before`, запросить approval;
5. если `ToolSafety::ReadOnly`, разрешить;
6. если `WritesFiles` или `RunsCommands`, запросить approval;
7. если `Network`, `Dangerous` или tool неизвестен, запретить.

Пример:

```toml
[module_config.policy.codex_policy]
allow = ["search", "read_file", "git_diff", "request_user_input", "apply_patch", "write_file", "write_stdin"]
allow_sandboxed = ["shell", "exec_command"]
ask_before = ["shell", "exec_command", "request_permissions", "remember_fact"]
deny = []
```

Packaged `codex` profile разрешает workspace-scoped `apply_patch` и
`write_file` без отдельного approval: оба handler-а проверяют workspace
boundary до записи. Эскалированные `shell` / `exec_command`, изменение durable
memory и остальные явно перечисленные действия сохраняют approval boundary.

Такой профиль делает Codex-подобный hot path явным: read-only и bounded
workspace-write tools видны без approval, неэскалированный shell работает в
sandbox, approval-gated actions требуют интерактивный transport, а
network/dangerous tools не появляются у модели без явной правки config.

### Approval-gated grants и request_permissions

`policy-pack` регистрирует tool `request_permissions`: модель заранее просит
turn-scoped эскалацию (сейчас поддерживается только `escalated_exec` —
unsandboxed запуск `shell`/`exec_command`). Механизм генерический и живёт в
contracts (`TurnPermissionGrants`): если tool call прошёл через явный user
approval и его успешный результат содержит `metadata.granted_permissions`,
core мержит эти строки в гранты текущего хода и передаёт их в
`PolicyContext::granted_permissions`. `codex_policy` при виде гранта
`escalated_exec` пропускает эскалированные вызовы из `allow_sandboxed` без
повторного Ask.

Гранты не переживают ход: `RuntimeContext` создаётся на каждый ход заново.
Core учитывает `granted_permissions` только на approved-пути, поэтому
`request_permissions` обязан стоять в `ask_before` — сам approval и есть
выдача гранта. Approval этого tool-а не кэшируется ни между turns, ни внутри
одного turn. Tool в `allow`-списке выдать грант сам себе не может.

Субагенты изолированы структурно: дочерний контекст получает пустые
`turn_grants`, поэтому `escalated_exec` родителя не протекает в ребёнка, а
гранты, выданные ребёнку через его собственный approval, не видны
родительскому ходу.

## allow_all

`allow_all` разрешает все tool calls. Используйте его только для тестов или доверенного окружения.

## opencode_policy

`opencode_policy` поставляется тем же `policy-pack` и используется
экспериментальным named config `opencode` (`configs/opencode.config.toml`). Это порт
permission engine из OpenCode (`permission/index.ts` + `util/wildcard.ts`).

Порядок решения:

1. tool маппится в permission-группу через
   `module_config.policy.opencode_policy.groups` (например, `edit` покрывает
   `edit_file`/`write_file`); без группы permission = имя tool-а;
2. из аргументов вызова извлекаются patterns по `pattern_args` группы
   (`command` для bash-группы, `path` для file-групп); при
   `split_commands = true` составная команда разбивается по `&& || ; |`;
   если извлечь нечего — pattern `*`;
3. для каждого pattern берётся **последнее** правило из `rules`, совпавшее
   wildcard-ом и по permission, и по pattern (last match wins); без
   совпадений — `ask`;
4. любой `deny` запрещает вызов; иначе любой `ask` требует approval; иначе
   allow.

Wildcard: `*` — любая подстрока, `?` — один символ, матч по всей строке;
pattern с хвостом `" *"` матчит и голый префикс без аргументов
(`"git push *"` матчит `"git push"`).

Видимость tools: tool скрывается из surface только если последнее правило,
совпавшее по его permission (pattern не учитывается), — `deny` с pattern `*`.
Точечные deny по pattern оставляют tool видимым, запрет срабатывает на
вызове.

Пример (upstream build-агент: allow-by-default, точечные ask):

```toml
[module_config.policy.opencode_policy]
rules = [
  { permission = "*", action = "allow" },
  { permission = "read", pattern = "*.env", action = "ask" },
  { permission = "read", pattern = "*.env.example", action = "allow" },
  { permission = "bash", pattern = "git push*", action = "ask" },
]

[module_config.policy.opencode_policy.groups.bash]
tools = ["shell"]
pattern_args = ["command"]
split_commands = true
```

В отличие от `codex_policy` здесь нет sandbox-ветки и turn-scoped grants:
эскалированные вызовы (`with_escalated_permissions`) оцениваются теми же
правилами группы `bash`. Порядок правил значим — более специфичные правила
ставьте ниже общих.

## Interactive Exec Lifecycle

PTY registry `exec_command`/`write_stdin` остаётся process-wide как деталь
реализации, но numeric session id служит только locator-ом. Handle принадлежит
runtime session/thread/workspace: тот же thread может продолжить процесс между
turn'ами, а другой session, thread или workspace получает явную ошибку.
Завершённые sessions и sessions с idle age от 30 минут удаляет минутный
janitor; общий cap 16 сохраняет LRU-eviction. Cancellation активного вызова
убивает процесс и удаляет handle.

## Известные Ограничения Текущей Реализации

Это текущие gaps, а не целевое поведение:

- `process` SubagentRunner ограничивает concurrent leases semaphore-ом и idle
  residents глобальным LRU-cap, но не имеет строгого wall-clock TTL/janitor;
- collaboration records имеют session ownership и caps, но живут только в
  памяти процесса: после restart нет list/wait/resume прежних handles;

До устранения этих gaps не считайте process-subagent/collaboration handles
durable process isolation boundary. Внешний `workdir` допустим только для явно
эскалированного unsandboxed вызова и сам по себе isolation boundary не создаёт.

## Planned Rights Model

Table-driven права tools/modules пока не реализованы. Целевая форма должна
оставить пользовательскую модель простой:

```text
config -> роль агента -> режим прав -> подключённые модули -> права tools/modules
```

Для tools планируется config с решениями `hide`, `deny`, `ask`, `allow`,
`priority`, `timeout_ms` и per-tool output limits. `hide` влияет на model
request, `deny` остаётся execution guard, `ask` требует approval, `allow`
разрешает исполнение без approval. `ToolSafety` остаётся нижним safety floor:
config не должен тихо превращать command/network/dangerous tool в безопасный.

Для modules та же идея может появиться позже, но первый шаг должен быть по
tools, потому что они уже имеют `ToolSafety`, `ToolRegistry`, approval и
execution path. Package manager, marketplace, WASM и OS sandbox в этот шаг не
входят; единый внешний process-module protocol уже реализован.

## Правила Для Новых Tools

- Всегда задавать корректный `ToolSafety`.
- Валидировать входной JSON до выполнения действия.
- Для file tools проверять workspace boundary.
- Для команд и сети считать действие потенциально опасным.
- Добавлять тест на policy behavior, если tool пишет файлы, запускает команды или ходит в сеть.
- Не исполнять tool в обход `ToolRegistry`, mode-aware `ApprovalPolicy` и `ToolOrchestrator`.
