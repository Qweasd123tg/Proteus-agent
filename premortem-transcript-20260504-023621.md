# Premortem Transcript: Modular Agent

Timestamp: 2026-05-04 02:36:21 Europe/Moscow

## Context Gathered

**What is being premortemed**

`/home/qweasd123tg/Code/Agent` is a Rust-first modular coding-agent harness. The architecture is:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> Contracts -> Modules
```

The main invariant is:

```text
Core -> Contract -> Module Implementation
```

Core should not know concrete details of search, memory, model providers, tools, policy, patch algorithms, workflows, or renderers. New behavior should go through an existing slot or a deliberately added contract.

**Who it affects**

The immediate audience is the project owner and future agent/contributor workflows inside this repository. The practical stakeholders are users who want a useful local coding-agent, plugin authors extending slots, and future UI/client implementations over app-server.

**What success looks like**

Six months from now, the project is a useful coding-agent for real code work and a credible local platform where agent ideas can be added through config, contracts, and modules without rewriting core. The TUI/app-server path should support long coding turns, approvals, cancellation, session state, and debugging. Tests should prove both slot replaceability and end-to-end usefulness.

## Premortem Frame

It is six months from now. Modular Agent has failed. It did not become a dependable coding-agent or a credible modular platform. We are looking back and explaining why it died.

## Raw Premortem Failure Reasons

1. **Architectural modularity became a tax.** Every useful feature needed a new slot, ABI glue, docs, tests, and config, so development slowed while core still accumulated exceptions for real-world cases.

2. **The dylib plugin boundary was too fragile.** ABI/layout drift, a global `~/.agent/plugins` directory, no package manager, no sandbox, and weak version isolation made plugins a source of breakage instead of modularity.

3. **The project built a nice plugin architecture but not a strong coding-agent.** Repo understanding, edit/test loops, diff review, evals, and workflow ergonomics lagged behind the architecture and behind competing tools.

4. **The trust model did not survive real work.** Without sandboxing, secrets policy, network gates, and centralized workspace enforcement, one bad tool/plugin/config could make the agent dangerous or untrusted.

5. **App-server/control-plane and TUI stayed demo-grade.** Cancellation, approvals, durable session state, reconnects, long turns, and protocol compatibility remained flaky.

6. **Documentation drifted away from code.** Implemented/planned mixed together, plugin lists and config examples aged, and future contributors followed attractive but incorrect rules.

7. **Tests gave false confidence.** Module-swap and unit tests proved contracts existed, but did not catch installed-plugin integration, real model/tool workflows, UX regressions, security bypasses, or coding-loop quality.

8. **Scope split the project in half.** It tried to become both a useful local coding-agent and a universal agent platform, and did not finish either to the level where users would choose it.

## Agent Deep Dives

### 1. Architectural Modularity Became A Tax

**Failure story**

Через несколько месяцев команда начала добавлять реальные функции быстрее, чем успевала оформлять их как слоты. Любая новая полезная способность - нестандартный memory policy, особый tool flow, альтернативный patch applier, экспериментальный renderer - требовала не только реализации, но и нового contract, ABI glue, manifest, конфигов, тестов на заменяемость и документации. В итоге простые изменения стали слишком дорогими, а roadmap начал копиться в backlog из "инфраструктурных" задач вместо продукта.

Параллельно core начал "протекать" исключениями. Чтобы не плодить ещё один slot ради маленького кейса, разработчики стали встраивать знания о конкретных плагинах и их ограничениях в runtime, app-server и UI. Снаружи система формально оставалась модульной, но фактически ключевая логика и спецслучаи закрепились в core. Это сломало обещание заменяемости: плагины уже нельзя было менять без каскадных правок, а модульность превратилась в налог на каждую итерацию.

**Underlying assumption**

Предполагалось, что универсальные contracts и slots всегда дешевле, чем точечные интеграции, даже для малых и быстро меняющихся возможностей.

**Early warning signs**

- Новые фичи регулярно начинаются с фразы "сначала нужно добавить ещё один slot/manifest/ABI glue", и это занимает больше времени, чем сама бизнес-логика.
- В `core` и `runtime` появляются `if plugin_id == ...`, `match` на конкретные реализации или temporary fallback-и, которые никто не удаляет.

### 2. Dylib Plugin Boundary Was Too Fragile

**Failure story**

Через 6 месяцев пользователи начали ставить несколько плагинов из разных веток и сборок: старый `sqlite-memory`, новый `renderer-pack`, локально собранный `file-tools`. Формально всё грузилось из `~/.agent/plugins`, но ABI/layout уже слегка разъехались: DTO изменился, `abi_stable` сигнатуры остались похожими, а runtime получал некорректные данные или падал на границе вызова. Ошибки выглядели как случайные: то не открывается сессия, то renderer ломает event stream, то tool call возвращает мусорный статус.

Потом выяснилось, что глобальная папка плагинов смешивает окружения разных проектов. Один агент ожидал policy/plugin версии `v0.3`, другой был собран под `v0.5`; `install.sh` перетирал dylib без lockfile, manifest compatibility не проверялся строго. Пользователь обновлял один проект и ломал другой.

Без sandbox и изоляции любой panic, segfault или зависание plugin code валили core или блокировали runtime. Вместо "модульной платформы" получился режим "если что-то сломалось, удали всё из `~/.agent/plugins` и пересобери". Команда начала обходить plugin boundary встроенными fallback-ами, и главный инвариант Core -> Contract -> Module Implementation стал номинальным.

**Underlying assumption**

Мы предположили, что `abi_stable` и дисциплина contracts достаточны для практической совместимости, хотя реальная эксплуатация требует version resolution, isolation, capability checks и воспроизводимой установки.

**Early warning signs**

- После `install.sh` или смены ветки тесты проходят, но запуск агента ломается только на машине с уже установленными старыми плагинами в `~/.agent/plugins`.
- Баг-репорты начинают лечиться советом "очисти plugin dir и пересобери", а не точной диагностикой несовместимой версии или manifest mismatch.

### 3. Plugin Architecture Outran Coding-Agent Quality

**Failure story**

Через 6 месяцев проект выглядел архитектурно зрелым, но в реальной работе проигрывал другим coding agents. Core, contracts и plugin slots были аккуратно разнесены, однако это не дало главного: агент не умел достаточно хорошо понимать репозиторий, устойчиво вносить правки и проверять их. `repo_aware` context остался полезным, но недостаточно сильным на больших кодовых базах, а edit/test loop так и не стал естественной частью workflow. Пользователь видел модульность, но не чувствовал ускорения.

Команда продолжала расширять plugin architecture, добавляла варианты workflow и policy, но ergonomics оставались вторичными. `coding.single_loop` и `coding.plan_execute_review` не закрыли самый частый сценарий: быстро найти релевантные файлы, сделать дифф, прогнать тесты, понять регрессии и повторить цикл без ручной координации. В итоге проект казался "правильным", но не "полезным": claims про coding-agent не подтверждались повседневным опытом.

Когда начали сравнивать с конкурентами и внутренними ожиданиями, стало ясно, что отсутствие eval harness, слабый diff review и недостроенный test runner не были второстепенными пробелами. Это были центральные причинно-следственные разрывы. Архитектура помогла расширяемости, но не ускорила итерацию над кодом.

**Underlying assumption**

Предполагалось, что хорошая модульность сама по себе быстро приведёт к сильному coding-agent поведению.

**Early warning signs**

- Пользователи регулярно обходят агент руками: сами открывают diff, запускают тесты и объясняют контекст вместо того, чтобы делегировать весь цикл.
- Roadmap стабильно сдвигает `repo_aware`, diff/test runner и evals, а новые plugin slots появляются быстрее, чем заметно растёт качество решения реальных задач.

### 4. Security And Trust Failed

**Failure story**

Через 6 месяцев агент начали запускать в реальных рабочих репозиториях, но модель доверия оказалась слишком хрупкой. Core считал `ToolSafety`, `PermissionMode` и `ApprovalPolicy` достаточными, однако фактическая защита файловой системы была размазана по tool/plugin-реализациям. Один сторонний или плохо написанный plugin обошел workspace-boundary, потому что core не навязывал invariant централизованно. В результате агент смог читать или менять файлы за пределами ожидаемого workspace.

Параллельно `auto`/`normal` режимы стали восприниматься пользователями как безопасные, хотя `allow_all`, shell-tool и отсутствие network/secrets policy превращали конфигурацию в скрытый trust bypass. Агент мог выполнить команду, утянуть токены из окружения, обратиться в сеть или изменить чувствительные файлы без понятной границы ответственности. Даже если это случилось один раз, доверие сломалось: пользователи больше не понимали, какие действия реально ограничены, а какие просто "обещаны" plugin-кодом.

Главный провал был не только в уязвимости, а в недоказуемости безопасности. Нельзя было уверенно сказать: "в этом режиме агент физически не может выйти за workspace, прочитать secret или отправить данные наружу". Поэтому проект стал непригоден для настоящей разработки, где цена ошибки выше демонстрационного сценария.

**Underlying assumption**

Предполагалось, что добровольная дисциплина plugin-авторов и корректная конфигурация policy достаточно надежны, чтобы заменить централизованное enforcement в core/runtime.

**Early warning signs**

- Security-инварианты описаны в документации, но не закреплены общими тестами на malicious/misconfigured plugin, shell escape, symlink traversal, env secret access и network egress.
- "Безопасные" режимы зависят от выбранного `ApprovalPolicy`/plugin behavior, а не от невозможности выполнить опасное действие на уровне core или OS boundary.

### 5. App-Server And TUI Stayed Demo-Grade

**Failure story**

Через 6 месяцев проект формально жив, но фактически не годится для длительной работы. `agent-tui` и `app-server` остались демонстрационной связкой: короткие запросы проходят, но при реальных coding turns всплывают гонки состояния, потеря фокуса, неконсистентные approvals и ломкая отмена/interrupt. Пользователь видит "вечные спиннеры", зависшие turns и не может уверенно понять, что уже выполнено, что отменено, а что ещё в очереди.

Контрольная плоскость так и не стала устойчивым контрактом. JSONL/DTO и event-log не были доведены до стабильности, поэтому внешние клиенты и тесты на совместимость постоянно ломаются при мелких изменениях. Durable session metadata, очередь approvals и восстановление после прерываний работают частично, а значит длинные сессии нельзя безопасно продолжать после restart, reconnection или ошибок транспорта.

В итоге UI не защищает runtime, а лишь маскирует его нестабильность. Команда тратит время на ручное отлавливание edge cases вместо наращивания возможностей, доверие к tool- и approval-потоку падает, и проект воспринимается как прототип без операционной надёжности.

**Underlying assumption**

Мы предположили, что тонкий transport-слой и постепенное "дотягивание" протокола будут достаточны для долгих turns, хотя для этого нужен жёсткий, версионируемый control-plane контракт.

**Early warning signs**

- Пользователи начинают регулярно сообщать: "turn завис", "approve/cancel сработал не на тот шаг", "после reconnect состояние не совпадает с UI".
- Количество регрессионных багов вокруг JSONL/event-log/session restore растёт быстрее, чем покрытие стабильными integration tests.

### 6. Documentation Drifted Away From Code

**Failure story**

Через полгода документация проекта стала выглядеть убедительно, но перестала быть надежным источником истины. В `README`, `docs/modules.md`, `docs/configuration.md` и `MODULAR_AGENT_SPEC_RU.md` остались старые списки slots, plugins и примеры конфигов, а новые изменения в коде уже жили отдельно. В результате contributors и агенты начали принимать решения по красивой, но устаревшей картине: добавляли модули не туда, переиспользовали неверные слоты и ссылались на фактически несуществующие или уже измененные механизмы.

Постепенно смешались `implemented` и `planned`. Спецификация перестала быть контрактом и стала набором пожеланий, а документация по инерции описывала "как должно быть", а не "как есть". Это породило ложную уверенность в совместимости, сломало onboarding и увеличило стоимость изменений: каждую правку приходилось проверять вручную по коду, потому что docs больше не помогали, а мешали.

В какой-то момент команда начала обходить документацию вовсе. Новые участники полагались на код, старые продолжали цитировать doc-страницы, и репозиторий распался на две версии реальности. Это особенно болезненно для модульного каркаса: если границы slot/contract/policy/renderer описаны неверно, архитектурная дисциплина уходит первой.

**Underlying assumption**

Предполагалось, что документация будет автоматически оставаться синхронной с кодом без явного процесса верификации и разграничения `implemented`/`planned`.

**Early warning signs**

- В PR-ах и ревью все чаще появляются фразы вроде "в документации написано иначе" или "это уже не соответствует коду".
- Новые contributors задают одни и те же вопросы про slots, plugin lists и config keys, хотя ответы якобы уже есть в docs.

### 7. Tests Gave False Confidence

**Failure story**

Через 6 месяцев проект выглядел "зелёным" по CI: `module_swap.rs` подтверждал заменяемость slots, unit-тесты plugin adapters и SSE parsers проходили, plugin crates тестировались изолированно. Но в реальных установках плагины из `~/.agent/plugins/` ломались на ABI/manifest/config несовместимостях, которые не покрывались workspace-тестами. Core формально не знал деталей модулей, но фактическая интеграция разваливалась на границе загрузки, регистрации, policy и tool execution.

Первые пользователи запускали coding workflow и получали нестабильное поведение: лишние правки, бесконечные tool loops, непредсказуемые approval prompts, плохие recovery paths после ошибок модели или shell-tool. Тесты доказывали, что contracts существуют, но не доказывали, что end-to-end сценарий "модель -> контекст -> tool -> patch -> tests -> renderer/UX" пригоден для работы.

Хуже всего, security regressions проходили незамеченными. Отдельные проверки `ApprovalPolicy`, `ToolRegistry` и `ToolSafety` были зелёными, но интеграционные workflow обходили ожидаемые choke points через неправильную wiring-комбинацию или plugin fallback. В итоге проект был архитектурно аккуратным, но операционно ненадёжным.

**Underlying assumption**

Мы считали, что проверка contracts и swap-ability автоматически означает практическую корректность installed-plugin workflows, UX, safety и качества coding loop.

**Early warning signs**

- Нет регулярного eval/integration harness, который запускает реальные coding-loop задачи и фиксирует `success/fail`, passed tests, model/tool calls, approvals, duration, tokens/cost, changed files, unnecessary edits и failure reason.
- Баги пользователей воспроизводятся только вручную через конкретную комбинацию config + installed dylib plugin + model/tool workflow, а не через `cargo test --workspace` или plugin-local tests.

### 8. Scope Split The Project In Half

**Failure story**

Через 6 месяцев проект застрял между двумя целями. С одной стороны, нужно было быстро довести до стабильности локального coding-agent, который реально помогает писать, проверять и изменять код. С другой стороны, команда и документация начали тянуть в сторону универсальной agent platform: слоты, ABI, плагины, memory, workflows, renderers, policy, MCP, расширяемость. В результате приоритеты размылись, а время ушло на архитектурную подготовку вместо доведения ядра до уровня, который регулярно используют.

Пользовательский опыт остался недостаточно сильным, чтобы вытеснить существующие инструменты: слишком много сборки вокруг конфигов и контрактов, слишком мало законченного сценария "открыл репозиторий, получил полезный результат". Одновременно platform-guarantees тоже не созрели: интерфейсы менялись, документация опережала реализацию, а слоты и плагины обещали больше, чем система могла надежно обеспечить. Итогом стал проект, который выглядел умно на бумаге, но не стал ни незаменимым рабочим инструментом, ни убедимой платформой.

**Underlying assumption**

Мы предположили, что один небольшой проект сможет параллельно довести до зрелости и прикладной UX, и общую платформенную архитектуру без жесткого сужения scope.

**Early warning signs**

- Растет число документов и контрактов, но реальные end-to-end сценарии не становятся заметно лучше и стабильнее.
- В issue/PR обсуждениях все чаще спорят о будущей extensibility и абстракциях, а не о том, как сократить путь пользователя до полезного результата.

## Synthesis

### The Most Likely Failure

The most likely failure is **plugin/platform work outrunning practical coding-agent quality**. The project already has strong architecture language and many slots, but the roadmap itself admits the missing center: repo-aware context, diff/test runner behavior, eval harness, phase configuration, and control-plane maturity. If priority remains tilted toward extensibility, the agent can become architecturally correct while still being slower than doing the work manually.

### The Most Dangerous Failure

The most dangerous failure is **security and trust collapse**. One workspace escape, secret leak, dangerous shell/network action, or plugin crash in a real repository would damage confidence more than a mediocre workflow would. This is especially dangerous because current safety depends partly on plugin discipline and config correctness, while the strongest safety claims need centralized enforcement or OS/process isolation.

### The Hidden Assumption

The hidden assumption is that **clean modular contracts will naturally produce a useful, safe, and stable agent**. They will not. Contracts preserve boundaries, but usefulness comes from measured end-to-end behavior, trust comes from enforceable invariants, and stability comes from reproducible installation plus protocol tests.

### The Revised Plan

1. **Freeze new slots unless an eval proves the need.** For the next 4-6 weeks, only add a slot/ABI extension when it directly fixes a failing end-to-end coding scenario. Everything else should be a workflow/config/tool improvement inside existing boundaries.

2. **Define one golden coding profile and make it excellent.** Treat `agent.coding.example.toml` plus the standard plugin pack as the product path. It should support: repo-aware context, patching, shell/test execution with approval, readable diff/result reporting, cancellation, and resume.

3. **Build the eval harness before adding more platform surface.** Start with 5 repository tasks: repo understanding, focused edit, failing test repair, approval/security refusal, and long-turn cancel/resume. Record success/fail, changed files, tests, tool calls, approvals, duration, tokens/cost, and failure reason.

4. **Make plugin installation reproducible.** Add strict `doctor` checks for plugin contract version, manifest/library path, duplicate ids, stale global plugins, missing standard plugins, and current config compatibility. Add a per-project or profile-scoped plugin directory option before encouraging multiple projects to share one `~/.agent/plugins`.

5. **Move security from promises to enforcement.** Add malicious/misconfigured plugin tests and central host checks where possible: canonicalize workspace paths before file-like operations, protect env secrets by default for process tools, deny network tools unless explicitly enabled, and ensure `auto` cannot be weakened by policy/config.

6. **Stabilize the control-plane contract.** Add protocol/state-machine tests for submit, stream, tool call, approval request/resolve, cancel, timeout, disconnect, reconnect, resume, and shutdown. Treat app-server DTO changes as compatibility-sensitive.

7. **Make docs verifiable.** Add a small inventory check that compares workspace plugin members, `plugin.toml` manifests, module ids from `modules list`, and documented plugin lists/config examples. Keep `implemented` and `planned` sections visibly separate.

### Pre-Launch Checklist

1. `agent doctor --strict` detects stale, missing, incompatible, duplicate, and globally shadowed plugins on a machine with an old `~/.agent/plugins`.

2. The golden coding profile passes at least 5 end-to-end eval tasks with recorded tool calls, approvals, changed files, test results, duration, tokens/cost, and failure reason.

3. Security tests cover malicious plugin path traversal, symlink escape, shell/env secret access, network egress defaults, `allow_all` misuse, and `auto` mode safety floors.

4. App-server/TUI integration tests cover long turn streaming, approval queue behavior, cancel/timeout, reconnect/resume, and session metadata consistency.

5. Documentation inventory checks fail when plugin lists, config examples, slot ids, or `implemented`/`planned` status drift from code.

