# Тестирование

Тест полезен, когда ловит конкретный дефект поведения или контракта.
Для обратимой низкорисковой правки не нужен отдельный тест, повторяющий
implementation. Сначала используйте или расширьте существующий сценарий.
Один инвариант проверяйте на нескольких слоях, только если каждый слой ловит
свой источник ошибки.

Не закрепляйте расположение кода, слова в исходниках, фразы настраиваемого
prompt или число компонентов в примере. Строгую схему проверяйте неизвестным
полем текущего формата; отдельные тесты для каждого удалённого имени не нужны.
Разбор specs reference tools уже проверяет
`conformance::aggregate_tool_module_lists_and_invokes_real_tools` через модуль.
Отдельный spec-тест оправдан собственной semantics, например `ToolSafety`.

## Стандарт Изменения

Для существенной правки определите проблему, ожидаемый результат и затронутую
границу. Выберите существующий test target; новый regression нужен, когда
существующее покрытие не ловит конкретный дефект.

После правки:

1. Выполните проверки по матрице ниже.
2. Обновите ближайший справочник и примеры, если изменилось поведение.
3. Проверьте `git diff --check` и создайте отдельный commit.
4. В результате перечислите выполненные проверки и непройденные применимые
   проверки с причиной.

Не запускайте подряд focused, package и workspace suites, если последний
набор уже включает нужные сценарии. Успешную проверку повторяют после новых
затрагивающих её изменений или для разбора конкретной ошибки.
Если полный прогон завершился с отдельными failures и изменены только эти
tests/fixtures, повторите упавшие targets. Успешные targets сохраняют evidence;
повторный workspace нужен при новой общей production-правке.
CI отключён; проверки выполняются локально. Manual dogfood — добровольная
[диагностика](dogfood-gate.md), без обязательного места в последовательности.

## Evidence Matrix

Матрица выбирается по изменённому поведению. Каталог targets ниже помогает
найти проверку; он не является списком команд для каждого изменения.

| Изменение | Достаточная проверка |
|---|---|
| Документация, комментарии | Содержание, локальные ссылки, `git diff --check` |
| Скрипты сборки приложения | `node --test clients/app/scripts/build.test.mjs`, затронутый реальный build; повторный запуск проверяет пропуск этапов |
| Prompt/config без schema change | Загрузка затронутого профиля; init/install только при изменении упаковки |
| Чистка tests | Изменённые test targets и сохранённые behavior checks |
| Локальный helper | Затронутый test target |
| Общий DTO/contract | Producers/consumers и полный Rust gate; затронутые клиенты отдельно |
| Assembly/config wiring | Plan → registry/topology, atomic reload, `module_swap`, `doctor` |
| Process protocol/slot adapter | Protocol/conformance, реальный модуль, `module_swap`; failure/restart/cancel по изменению |
| Module implementation | Module target; реальный invocation при изменении process boundary или side effects |
| Tool/policy | Поведение tool и общий registry/policy/approval path по изменённой ветке |
| Workflow/runtime | Canonical journal и replay в поддержанной границе; cold history/terminal при изменении recovery |
| Agent control | Real process peers, адресная доставка/отмена и независимые child configs |
| HTTP/session | Затронутые handlers; reconnect/cold history/auth/SSE при изменении этих границ |
| MCP/ACP | Реальный stdio peer; prompt/tools/approval/cancel и cold history по изменению |
| Интерфейс приложения и диагностика | Затронутые Rust/Node tests и `trunk build`; browser smoke при UX change |
| UI extensions | Contract/lifecycle tests; реальный browser/agent API при изменении интеграции |
| Desktop launch/package | Backend lifecycle, portable build и native smoke; при изменении графического запуска — Linux/NVIDIA default и явный override переменных окружения до GTK, native Wayland с аппаратным ускорением |

Полный Rust gate нужен для общих contracts, runtime wiring, зависимостей,
изменений взаимодействующих crates, интеграции и release:

```bash
cargo fmt --all --check
./scripts/test.py full
git diff --check
```

Для локальной правки используйте, например,
`./scripts/test.py -p proteus-core --lib <filter>` или
`./scripts/test.py -p proteus-core --test <target> <filter>`.
`cargo check` не заменяет поведенческие tests.

## Быстрая Локальная Проверка

`scripts/test.py` принимает обычные Cargo arguments с явным `-p`, а `full`
выбирает весь workspace. Для локального module/helper выбирайте package или
filter из матрицы; полный прогон не нужен после каждого изменения.

Runner один раз готовит свежий reference-модуль и передаёт его path через
`PROTEUS_TEST_REFERENCE_MODULE`. Core fixtures не запускают Cargo и не угадывают
старый binary в `target/`. Full использует executable из того же `cargo test
--no-run`; focused Core использует отдельный cache `target/test-module`, чтобы
узкий feature graph не перезаписывал workspace artifacts. Обычные package
tests без Core не собирают модуль.
Для installed smoke или явно подготовленного модуля можно задать его path в
`PROTEUS_TEST_REFERENCE_MODULE`: focused runner использует этот executable без
сборки. Свежесть такой явной привязки обеспечивает вызывающий сценарий; missing
file — ошибка. Full всегда получает модуль из своего Cargo artifact stream.

Defaults runner-а: два build jobs, четыре test threads, loopback в `NO_PROXY`
и отключённые Python bytecode caches. Явные environment settings сохраняются.
После изменения build settings первая сборка заполняет cache заново; скорость
тёплого прогона измеряется отдельно от этой разовой стоимости.

Dev/test сохраняют line tables для backtraces без тяжёлой variable/type debug
информации. Для отладки переменных можно задать `CARGO_PROFILE_DEV_DEBUG=2`
и `CARGO_PROFILE_TEST_DEBUG=2`; смена этих settings требует пересборки.
На `x86_64-unknown-linux-gnu` настроен установленный `lld` через `cc`;
для Linux development требуется executable `ld.lld`. Остальные targets,
включая WASM, используют свои linker settings.

## Каталог Проверок

Имена в таблицах — package, integration target или Rust module filter.
Точные cases и fixtures находятся в указанных исходниках; отдельный пересказ
каждого сценария в справочнике не поддерживается.

### Process И Modules

| Граница | Target / исходники |
|---|---|
| Framing, child lifecycle, bounded transport | `proteus-process-host`, включая собственный runner `tests/session.rs` |
| RPC envelope, wire ID, handshake, export identity, callback authority, multiplexing | `proteus-module-protocol` (`v3::frame`, `v3::wire_id`), `tests/broker_v3.rs` |
| Подмена implementation, structural absence, shared component и failure/restart | [`proteus-core --test module_swap`](../../crates/proteus-core/tests/module_swap.rs) |
| Reference exports, descriptors, tools и callbacks | [`proteus-reference-module --test conformance`](../../modules/reference/process-module/tests/conformance.rs) |
| Ordered hooks, actual effect и следующий живой turn после interruption, replay Success/Error | [`proteus-core --test hook_runtime`](../../crates/proteus-core/tests/hook_runtime.rs) |
| JS/TS hook SDK, перенос handlers, multiplexing и targeted cancel | `node --test examples/modules/hook-process/tests/*.test.mjs`; настоящие slot/journal/replay — `hook_runtime::js_ports` и `hook_runtime::review` |
| Model process, catalog/quota и cancellation | `proteus-core --test model_process`, `proteus-reference-module --test model_exports` |
| Execution attribution и операции без chat identity | `proteus-core --test execution_boundary`, `core::runtime::tests::execution`, `core::bound_model`, `core::bound_tools` |
| Один process с callback-связанными exports и journal/replay | `proteus-reference-module --test topology_journal` |
| Patch transaction через заменяемые реализации | `proteus-reference-module --test patch_transaction` |

Новый callback проходит единую authority table и dispatcher всего slot.
Проверяются разрешённый вызов и отказ из другого slot; привилегия конкретного
reference `module_id` не является корректным evidence.
Protocol tests покрывают malformed/missing/unknown fields, неверные ids и
exports, запрещённые методы, oversized frames, child exit, module error,
timeout/cancel. Producer, consumer и fixtures меняются вместе; старый reader
ради compatibility не добавляется.

Conformance CLI без probe проверяет handshake, но не поведение slot.
Команды для внешних Python examples находятся рядом с ними:
[search](../../examples/modules/search-process/README.md),
[compactor](../../examples/modules/compactor-process/README.md),
[workflow](../../examples/modules/agent-worker/README.md).

### Runtime, Journal И Replay

Для runtime-поведения источник истины — canonical journal:

- prompt replay сверяет post-shaping model request;
- workflow replay сверяет orchestration на записанных outcomes;
- cold `/history` подтверждает durable projection;
- `TurnSettled` фиксирует terminal state.

Workflow replay поддерживает root `Success`/`Error` в описанной
[границе](../architecture/canonical-turn-data.md#workflow-replay-v0).
Внешние `Canceled`/`Timeout` проверяются через settlement и cold history.
Replay должен обходиться без исходного model/tool effect и не менять source
journal. Для checkpoints проверяются history revision, точный
`execution_call`, выбранные results и их положение относительно model/tool
facts; совпавший final output не заменяет эти проверки.

| Граница | Target / filter |
|---|---|
| History, settlement, recovery, redaction | `proteus-core --lib core::session_journal`, `core::session_store`, `core::runtime` |
| Единственный OS writer | `proteus-core --test session_writer_lock` |
| Prompt/workflow replay | `proteus-core --lib core::prompt_replay`, `core::workflow_replay` |
| Codex stream/retry/tool progress/cold resume | `proteus-reference-module --test codex_model_resume` |
| Local compaction и совместимость compactors | `codex-compactor`, `context-pack`, module targets `codex_compaction`, `compactor_interop` |
| Model-free controller | `coding-workflow project_check`, module target `project_check_workflow` |
| Frozen admission и run intent | `core::runtime::tests::snapshot_atomicity`, `coding-workflow intents` |
| Изображения: process workflow, cold resume, capability refusal, replay | `proteus-core --test execution_boundary images::`, provider encoding в `model-pack`, сохранение parts в `codex-compactor` |
| Process peers и их tool surfaces | Core targets `process_agent_control`, `process_agent_pool` |

Storage recovery проверяется отдельно от steady-state append: полный scan
выполняется при открытии writer, а не при каждом record. Redaction сохраняет
JSON schemas и удаляет credentials из metadata/arguments. Сокращение проверок
не является основанием убирать `sync_data`, checkpoint или policy contracts.

Pinned upstream revision, подтверждённые срезы и ограничения реконструкции
собраны в [codex-baseline.md](codex-baseline.md) и module `UPSTREAM.md`.
Новый upstream output сначала классифицируется как parity change,
unsupported capability или намеренный divergence. Snapshot не обновляется
вслепую. Replay проверяет эквивалентность; качество и расход на живых задачах
проверяются отдельным eval по [roadmap](../product/roadmap.md).

### Config, Tools И Transport

| Граница | Target / filter |
|---|---|
| TOML/JSON, includes, profiles | `proteus-core --test config_profiles`, `core::config`, `core::assembly` |
| Выбор context implementation | `proteus-reference-module --test context_profile_swap` |
| CLI dispatch до загрузки config | `proteus-core --test cli_dispatch` |
| MCP discovery/result mapping/cancel/restart | `proteus-core --test mcp_client` |
| ACP prompt/approval/cancel, model/mode selectors | `proteus-core --test acp_server`, `--lib app_server::acp` |
| HTTP адресация, очереди, SSE и cold reads | `proteus-core --lib app_server::http` |
| Live projections и client revisions | `app_server::events`, `app_server::turn_progress`, `proteus-app-common` |
| OAuth/Responses/Anthropic mapping и retries | `model-pack`; module targets `auth_commands`, `model_exports` |
| Terminal processes | `shell-tool`; модуль `codex_model_resume terminal::` |

Изменённые configs должны загружаться через текущий reader. При изменении
schema обновляются все tracked producers/consumers/examples. Для проверки
сборки без model request используйте `doctor`, `modules list` или `tools list`
на representative profile; reference-модуль должен быть собран и доступен
в `PATH`.

Tool проверяется по собственной semantics: input, safety, visibility,
allow/ask/deny, path, timeout/cancel или output limits — когда эта ветка
затронута. Process tool проходит `list`/`invoke`, а runtime-вызов — общий
`BoundTools -> ToolRegistry -> policy`. Workflow вызывает tools через host,
а не исполняет команды самостоятельно.

Fixtures используют локальные HTTP/stdio peers, без live credentials.
Runner явно собирает reference-модуль до Core tests; это не production dependency
Core на reference crates. Shell fixtures разбирают JSON-RPC id JSON-парсером,
не полагаясь на порядок ключей.

### Клиенты И Установка

Интерфейс приложения и встроенная диагностика исключены из root workspace.
Их пакеты собираются через Trunk:

```bash
node clients/app/scripts/dependencies.mjs
(cd clients/app/ui && env -u NO_COLOR trunk build --locked)
(cd clients/app/diagnostics && env -u NO_COLOR trunk build --locked)
```

`cargo check` внутри клиента не заменяет Trunk из-за различий target/features.
При изменении shared client DTO проверяются оба consumer-а. При локальной
правке одного клиента второй собирать не требуется.

CSS/JS/картинки проверяйте на уже работающем `./scripts/desktop.sh dev`:
обновление ресурсов не требует Rust/WASM или release-сборки. Новая переносимая
сборка нужна при проверке упаковки, native startup или по запросу владельца.
Не запускайте её после каждой правки интерфейса. Общие Rust gates выбирайте
по затронутой границе, а не по факту запуска приложения.

Скрипты сборки проверяются отдельным Node-сценарием с временным репозиторием
и подменёнными компиляторами: выбор этапов, повторный запуск, изменение CSS,
embedded config, восстановление WASM и permissions, ошибка этапа. Этот сценарий
не заменяет реальную сборку при изменении compiler flags или упаковки assets.

- Browser/Node commands и покрытие UI extensions:
  [ui-extensions.md](../guides/ui-extensions.md#проверка).
- Desktop Rust/build/native smoke:
  [desktop.md](../guides/desktop.md#проверка).
- Изображения в реальном UI и WebKitGTK: `python3 clients/app/ui/tests/extensions_browser.py --images-only` после Trunk и сборки backend/reference module. Используется локальный provider fixture; проверка не подтверждает качество живой vision-модели.
- Изолированная проверка installer: `./scripts/install-smoke.sh`.
  Она сама собирает и устанавливает оба executable во временные каталоги,
  проверяет fake turn, внешний Python component и process peers. Отдельная
  предварительная установка для этого smoke не нужна.
  `sh scripts/install-smoke-env.sh` выполняет тот же сценарий с подставным
  внешним `PROTEUS_CONFIG_PATH` и проверяет, что его файл не изменился.
  Smoke снимает этот override и направляет `init` в явный временный config.
  Process fixtures привязаны к установленным executable из `current`:
  shell launchers требуют пользовательский env, который component fixture
  намеренно не наследует. Launchers проверяются отдельными CLI этапами smoke.

Для docs-only правки этих руководств достаточно проверки содержания и ссылок;
пересборка клиента или установка не требуется.

## Ordered Hook Evidence

Самостоятельный DCP Node component проверяется `npm test` в
`modules/reference/dcp`. Runner перед `full`, core integration tests и
`hook_runtime`/`module_swap` выполняет locked `npm ci --ignore-scripts` и build,
передаёт свежий путь через `PROTEUS_TEST_DCP_MODULE`; сборок внутри Rust tests нет.
`hook_runtime` связывает реальные hook/tool exports и scripted process model:
outgoing context отличается, cold history сохраняет оригиналы, replay `Success`
и `Error` не вызывает model/DCP и не меняет package-owned state. Live-model
dogfood/eval проверяется отдельно: scripted evidence не доказывает полезность
или качество summary на повседневных задачах.

## Slash-команды

`proteus-core --lib app_server::http::tests::slash` проверяет общий каталог,
prompt expansion и настоящую process-команду: allow/deny approval, idle admission,
cancel и cold journal без Turn/model call. `hook_runtime` проверяет DCP management
и replay без повторного изменения package state; `npm test` — оригинальные
stats/context/decompress handlers и восстановление view после restart.
User-only tool не попадает в model catalog и отклоняется model-request validation.

После Trunk и свежей debug-сборки backend/reference module:

```bash
python3 clients/app/ui/tests/extensions_browser.py --commands-only
```

Сценарий проверяет общий backend catalog, keyboard completion, служебные и DCP
команды без LLM, сохранение ошибочного черновика, обычный prompt Turn и `//`.

## Typed Hook Boundary

Для `hook/v4` проверяются strict DTO/response validation, одинаковая authority
без host callbacks, config order и отсутствие duplicate/unknown selection.
Boundary chain покрывает A→B/B→A, pre-effect failure, actual tool outcome
при post-effect failure, targeted cancellation и component restart. Config
builder roundtrip сохраняет order при изменении других slots. Изменение model
request/tool result проверяется canonical journal и workflow replay в
поддержанной границе; записанные transformed outcomes не применяются повторно.
