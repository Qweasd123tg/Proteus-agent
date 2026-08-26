# Pi и Proteus: решение о позиционировании

- Статус: research/decision input, не reference текущей реализации.
- Статус-обновление 2026-07-16: 30-дневный эксперимент не запускается —
  решение владельца; действующий курс зафиксирован в
  `docs/product/roadmap.md`
  («План: Месяц Гибкости»). Этапы 1–2 Pi-интеграции переиспользованы там
  как обычные задачи без experiment-обвязки.
- Архитектурный cleanup 2026-07-16 закрыл два кандидата из этой заметки:
  `MemoryPolicy`/`carry_forward` удалены, а
  `coding.codex_loop_diagnostic` retired с config migration на strict
  `coding.codex_loop`. Список ниже сохранён как датированное основание решения.
- Дата среза: 2026-07-13.

Эта заметка отвечает на неприятный, но конкретный вопрос: остаётся ли у
Proteus самостоятельная причина существования после знакомства с Pi, или
дальнейшая работа превращается в дорогую реализацию уже готового harness-а.

Решение о pivot пока не меняет `README`, `spec`, `scope` или `roadmap`. Сначала
гипотеза должна пройти ограниченный по времени эксперимент ниже.

## Проверенный Срез Pi

Сырые исходники находятся в локальном git-ignored каталоге
`examples/source/pi`. На другой машине этот каталог может отсутствовать,
поэтому evidence-ссылки ниже закреплены на точном upstream commit:

- upstream: <https://github.com/earendil-works/pi>;
- branch: `main`;
- commit: `8479bd84743e8889f728acb21a62794102db0529`;
- дата upstream commit: 2026-07-11;
- лицензия: MIT.

Основные проверенные поверхности:

- [`packages/ai`](https://github.com/earendil-works/pi/tree/8479bd84743e8889f728acb21a62794102db0529/packages/ai) —
  unified multi-provider API;
- [`packages/agent`](https://github.com/earendil-works/pi/tree/8479bd84743e8889f728acb21a62794102db0529/packages/agent) —
  model/tool loop, typed events и новый `AgentHarness`;
- [`packages/coding-agent`](https://github.com/earendil-works/pi/tree/8479bd84743e8889f728acb21a62794102db0529/packages/coding-agent) —
  CLI, sessions, compaction, extensions, packages, SDK и RPC;
- [`packages/tui`](https://github.com/earendil-works/pi/tree/8479bd84743e8889f728acb21a62794102db0529/packages/tui) —
  terminal UI;
- [`packages/orchestrator`](https://github.com/earendil-works/pi/tree/8479bd84743e8889f728acb21a62794102db0529/packages/orchestrator) —
  experimental supervisor независимых Pi RPC processes.

Planned-возможности Pi не считаются реализованными. Особенно это относится к
части lifecycle/recovery нового
[`AgentHarness`](https://github.com/earendil-works/pi/blob/8479bd84743e8889f728acb21a62794102db0529/packages/agent/docs/agent-harness.md)
и к experimental orchestrator.

## Короткое Решение

**Как ещё один универсальный extensible coding-agent Proteus больше не имеет
убедительной причины существовать.** Pi уже заметно сильнее как повседневный
терминальный инструмент и почти дословно закрывает исходный pitch
«адаптировать agent под себя без форка чужого CLI».

Продолжение Proteus оправдано только как ограниченная проверяемая гипотеза:

> Proteus — локальный policy-first runtime и экспериментальный стенд на Rust
> для безопасного реального использования и воспроизводимого сравнения
> архитектур coding-agent на одинаковых задачах, моделях и рабочих деревьях.

Отрицательная граница столь же важна:

> Proteus не должен догонять Pi по provider breadth, TUI, session UX или
> Pi Packages ecosystem.

Даже новая формулировка пока не является доказанным moat. Pi можно расширять
TypeScript hooks, встраивать через SDK/RPC и изолировать его tools/process через
micro-VM/container. Proteus должен выиграть измерением, safety invariant или
control-plane качеством, а не наличием похожей функции.

## Что Pi Уже Закрыл Лучше

| Область | Pi | Proteus | Решение |
|---|---|---|---|
| Providers и auth | Десятки providers, API keys и subscription login | OpenAI, Anthropic, compatible и fake | Не участвовать в гонке providers |
| Повседневный UI | Зрелый differential TUI | Dogfood web-клиент и простой CLI | Не строить TUI parity |
| Sessions | JSONL tree, branch, fork, clone, labels, import/export | Линейная history, canonical journal и resume | Не копировать tree UX без eval-потребности |
| Extensions | TypeScript hooks для tools, commands, provider payload, context, compaction и UI | Narrow traits + dylib ABI | Не продавать dylib как более удобную ecosystem |
| Distribution | npm/git Pi Packages, install/update/config/reload | Локальный `install.sh`, plugins при старте | Заморозить marketplace/package-manager идеи |
| Embedding | Interactive, print, JSON, RPC и SDK | CLI, HTTP/SSE и stdio app-server | Использовать Pi RPC как baseline, не переписывать его core |
| Базовый agent loop | Компактный production loop со steering/follow-up | Несколько сменных workflow plugins | Сохранять сменность только если она меняет измеримый результат |

Это означает, что multi-provider, tools, sessions, streaming, compaction,
plugins и subagents как отдельные пункты списка больше не отвечают на вопрос
«зачем существует Proteus».

## Где Гипотеза Proteus Ещё Имеет Смысл

### 1. Обязательный Safety Path

Proteus направляет model-callable actions через общий путь
`ToolRegistry -> ApprovalPolicy -> ToolOrchestrator -> Tool::invoke`, хранит
атрибуцию approvals и запускает non-escalated shell fail-closed через `bwrap`.
Pi прямо не имеет встроенной permission/sandbox системы. Gondolin оставляет Pi
и auth на host, но маршрутизирует built-in tools и `!` commands в micro-VM.
Docker и OpenShell изолируют весь Pi process. Extension также может добавить
собственный confirmation/policy gate.

Это реальное отличие, но с двумя оговорками:

1. dylib-плагины Proteus trusted и выполняются in-process; речь идёт о защите
   model-callable tool path, а не о sandbox произвольного plugin code;
2. Pi внутри полноценного container может дать более простую и сильную
   OS/process boundary, но это не per-action approval и RW mount рабочего дерева
   всё равно разрешает менять host files.

Поэтому сравнивать нужно не Proteus против голого Pi, а Proteus против Pi в
разумной sandbox-конфигурации.

### 2. Contract-Level Replaceability

Proteus имеет narrow slots, public contracts crate, config-selected module ids,
dylib ABI и swap regression tests. Pi имеет широкую и удобную in-process hook
surface, но не делает взаимозаменяемость search/context/policy/workflow/memory
системным product invariant.

Однако это преимущество хрупкое:

- несколько Proteus slots, включая search, memory policy, patch и compactor,
  пока имеют `none/stub + one` production implementation;
- ABI/JSON bridge создаёт заметную стоимость;
- новый Pi `AgentHarness` уже формализует turn snapshots, phases, pending
  durable writes, tool registry semantics и будущие hooks/recovery.

Слот имеет ценность только если внешняя реализация подключается без core-правок
и даёт повторяемо отличающийся outcome, cost, safety или latency.

### 3. Correlated Trace, Replay И Eval

Proteus уже хранит canonical journal с session/thread/turn/sequence,
request/response/tool records, config snapshots и compaction lineage.
App-server несёт correlated live events и inspection endpoints; web отображает
approvals/subagent attribution, а Inspector — topology.

Но текущий `eval report` — только агрегация trace. Пока нет one-command runner,
versioned corpus, scoring по tests/diff/cost и сравнительного отчёта, это не
преимущество, а незавершённая инфраструктура.

Storage часть кластера `canonical turn data -> replay -> eval` закрыта
2026-07-23. Следующим продуктовым доказательством остаются replay/eval runner,
versioned corpus и scoring; без них best-of packs являются более дорогой
версией Pi Packages.

### 4. Supervised Multi-Agent Control Plane

Proteus имеет first-class `SubagentRunner`, process/worktree isolation,
budgets, cancellation, approval origin, bounded process retention и
session-owned collaboration surface. Pi core сознательно не стандартизирует
subagents, но поставляет рабочий example extension и experimental process
orchestrator.

Текущий Proteus control plane process-resident и не restart-durable; в нём нет
fork/nesting/writer spawn, а process/plugin runners пока не имеют messaging
capability. Это задел для проверки, а не готовый multi-agent moat.

Простого наличия `spawn` недостаточно. Proteus должен доказать одно из двух:

- не менее 20% медианного ускорения либо 15 процентных пунктов success-rate
  gain на decomposable tasks при token overhead не выше 30%;
- либо pre-registered safety/control test, который Proteus проходит после
  одинакового fixed marginal effort для обоих backends, а Pi comparator — нет.

## Что Сохранить, Переосмыслить И Заморозить

### Сохранить И Проверять

- `ToolOrchestrator`, policy/approval contracts и fail-closed shell sandbox;
- `proteus-process-host` как reusable persistent stdio lifecycle primitive;
- canonical journal, correlated event envelope, config snapshots и app protocol;
- worktree/process subagent lifecycle, ownership, attribution и budgets;
- module swap/boundary tests;
- provider-neutral canonical model только как слой нормализации экспериментов.

### Переосмыслить

- app-server, web и Inspector — не отдельный chat product, а supervised
  control/eval plane;
- profiles/packs — не коллекция возможностей, а экспериментальные варианты с
  baseline, гипотезой, метриками и датой promotion/kill review;
- plugin ABI — внутренний механизм boundary discipline, не пользовательское
  преимущество над Pi extensions;
- `eval report` — нижний слой будущего comparative runner, а не готовый eval.

### Заморозить На Время Эксперимента

- новых providers и auth flows;
- TUI/CLI parity и cosmetic UI rewrite;
- marketplace, package manager, hot reload и новые plugin formats;
- session tree/fork/clone parity;
- новые memory/RAG/LSP/MCP capabilities;
- новые slots и compactor variants без заранее определённого A/B case;
- feature packs без versioned corpus и scoring.

### Не Удалять До Решения

Нельзя удалять подсистемы только из-за текущего эмоционального удара. После
эксперимента кандидатами на collapse/removal становятся slots с одной реальной
implementation, дублирующие stubs и ABI bridges, которые не участвовали ни в
одном измеримом swap. Удаление должно следовать данным, а не sunk cost и не
желанию начать заново.

Первые конкретные кандидаты для отдельного post-experiment решения:

- `coding.codex_loop_diagnostic`, когда structured trace/UI уже честно
  показывают terminal failure без отдельного workflow;
- heuristic `memory-pack::carry_forward`, если recall eval не подтверждает
  пользу последних 500 символов assistant reply;
- cosmetic `Renderer` slot после сохранения необходимых one-shot/admin
  форматов;
- interactive REPL после сохранения CLI-команд `doctor`, `modules`, `inspect`,
  `eval`, `server` и one-shot automation.

Это кандидаты, а не утверждённый план удаления.

Статус после review 2026-07-16: первые два кандидата выше уже закрыты —
diagnostic workflow id и heuristic memory policy удалены. Остальные пункты
остаются только candidate list, не активным планом удаления.

## Как Относиться К Pi Технически

### Этап 1: External Baseline, Не Новый Slot

Первый Pi adapter должен жить в eval/research path. До появления
protocol-neutral raw seam он может быть отдельным research driver; после этого
он использует lifecycle существующего `proteus-process-host`. Он нормализует:

- initial task, runtime version/commit, model/provider и workspace revision;
- workspace dirty state, system prompt/resource/tool manifests или hashes;
- reasoning/transport/API settings, package/extension set;
- sandbox profile, mounts, network policy и control mode;
- Pi `AgentSessionEvent`/RPC responses;
- итоговый diff, test result, duration, tokens/cost и failure reason.

Нельзя маскировать Pi как обычный Proteus `Workflow`: Pi исполняет собственные
tools и тем самым обходит `ToolRegistry`/`ApprovalPolicy`. Такой wrapper создаст
ложное впечатление общей safety boundary и испортит A/B.

Run record обязан явно указывать control mode, например:

```text
runtime = proteus_native | pi_rpc
tool_control = proteus_enforced | external_no_tools | external_unmediated | external_container
```

Новый public contract не нужен, пока нет второго external runtime adapter
(например, Codex). Сначала достаточно research runner и внутреннего
нормализатора.

### Этап 2: Ограниченный No-Tools Pi Как Существующий Subagent Slot

Минимальная runtime-интеграция не требует нового slot: Pi семантически является
целым дочерним model loop и может реализовать существующий `SubagentRunner` под
experimental id вроде `pi_rpc_reasoner`.

Первый slice обязан запускать pinned Pi executable без tools и project-local
executable resources. Процесс получает пустые per-run cwd и agent dir, а не
реальный workspace:

```text
PI_CODING_AGENT_DIR=<empty-per-run-agent-dir> PI_TELEMETRY=0 \
  pi --mode rpc --offline --provider <provider> --model <model-id> \
  --no-session --no-tools --no-extensions \
  --no-skills --no-prompt-templates --no-context-files --no-approve \
  --system-prompt <bounded-role-prompt>
```

Proteus заранее собирает bounded context через `repo_aware`, добавляет role
prompt и передаёт JSON-encoded текст дочернему Pi. Explicit system prompt не
должен обещать недоступные read/edit/bash tools.

No-tools означает только, что модель не может инициировать Pi tools. Это не
sandbox самого Pi executable: startup migrations, auth/provider code и model
network находятся вне Proteus `ToolOrchestrator`. Пустые cwd/agent dir защищают
реальный project/global config от migrations; `--offline` выключает startup
version/package/telemetry requests, но не model request.

Текущий `ProcessSpec` дополняет inherited environment. До spike ему нужен
generic `env_clear`/allowlist: дочерний процесс получает только необходимые
runtime variables, выбранный provider/model и scoped credential. По возможности
Pi запускается без mount реального workspace внутри внешней process sandbox.

Pi RPC также имеет out-of-band command `bash`, независимо от model tool list.
Adapter жёстко разрешает отправлять только `prompt`, `abort` и заранее
перечисленные read-only control messages; raw frame API остаётся trusted
plumbing и никогда не экспонируется model/config/user payload.

Одна Pi process session обслуживает ровно одного logical child: `--no-session`
отключает disk persistence, но не очищает in-memory conversation. Первый slice
создаёт fresh process на каждый child. Нельзя иметь concurrent prompts в одном
process.

Completion наступает только на `agent_settled`, а не на acceptance response,
`message_end` или `agent_end`. Timeout/cancel сначала отправляет `abort`, затем
убивает/reset-ит process, если settlement не подтверждён; stale events не могут
попасть в следующий run. Сохраняются max-frame limits и добавляются bounds на
event count, aggregate bytes и channel buffering.

Implementation объявляет role как `parallel_safe = false`,
`supports_collaboration = false` и не обещает worktree/mailbox semantics. Она
мапит cancellation, timeout, `max_total_tokens`, usage, status и bounded summary
в существующий `SubagentResult`. Иначе это не корректная реализация
`SubagentRunner`.

Текущий `ProcessHost::request()` нельзя использовать напрямую: он формирует
JSON-RPC `method/params/result`, тогда как Pi RPC использует `{id,type,...}`,
отдаёт acceptance response отдельно от terminal events и стримит id-less
events. Utility crate нужен protocol-neutral raw seam:

- `send_frame(Value)`;
- `recv_frame(timeout)` / `try_recv_frame`, где timeout сам не решает судьбу
  process, а adapter применяет описанный abort/kill protocol;
- явный `terminate/reset`;
- bounded receive buffering и environment allowlist;
- frame classification внутри Pi adapter, а не внутри process host.

Pi DTO не должны попадать в `proteus-core` или utility crate.

### Этап 3: Проверка Controlled Pi Tools

Если baseline полезен, отдельный короткий spike может заменить Pi tools
extension-ом, который отправляет tool requests в Proteus executor. Это даст
общий approval/trace path, но только ценой дополнительного protocol bridge.

Spike прекращается, если container/Gondolin решает задачу проще или если bridge
требует переносить Pi session/provider semantics в Proteus core.

### Этап 4: Control Plane Только После Двух Backends

Лишь после Pi и ещё одного external backend можно обсуждать общий
`ExternalAgentRuntime` contract. До этого новый slot нарушит собственное
правило Proteus «contract для класса заменяемого поведения, а не для одной
фичи».

## Threat Model И Лабораторная Граница

Основная заранее зарегистрированная гипотеза Proteus — не «общая безопасность»,
а integrity/observability model-callable action path:

- denied tool invocation не достигает spawn/write/network side effect;
- разрешение имеет правильные session/thread/turn/child attribution;
- cancellation и resource bounds не позволяют одному run управлять другим;
- host-visible forbidden effects обнаруживаются correlated trace и внешним
  oracle.

Pinned Proteus core/plugins и Pi executable считаются trusted. Эксперимент не
доказывает containment злого dylib/extension/runtime и не закрывает полностью
confidentiality/provider secret exfiltration через context/model adapter.
Availability проверяется только в заявленных timeout/cancellation/resource
границах. Эти ограничения входят в отчёт и не могут называться пройденной
«полной safety».

Все backends, включая stock Pi, запускаются внутри одинаковой внешней
одноразовой lab boundary с synthetic secrets. Каждый run получает свежую копию
pinned fixture/worktree. Stock Pi означает штатное agent behavior внутри этой
внешней защиты; Pi+sandbox означает выбранный до начала corpus конкретный
product sandbox profile внутри той же lab boundary. Реальный repo/host никогда
не используется для path/symlink/outside-workspace атак.

Forbidden effects наблюдает host-side oracle вне candidate sandbox: writes вне
разрешённого дерева, network при deny, process spawn после deny, cross-session
approval reuse и child/worktree contamination. Часть cases заранее
публикуется, часть остаётся held-out или автоматически мутируется.

## Ограниченный Эксперимент: 30 Дней

Окно проверки: 2026-07-13 — 2026-08-12. В течение окна parity/features
заморожены. Четыре недельных блока занимают 28 дней; последние два дня — только
на итоговый report и решение.

### Неделя 1: Данные И Corpus

- выбрать 12 versioned tasks: repo understanding, focused edit, failing-test
  repair, approval refusal, outside-workspace denial, symlink/path escape,
  cancel/resume и worktree child;
- определить canonical run record: input, workspace revision, model/config,
  runtime commit, prompts/resources/tools, reasoning/transport, sandbox,
  extensions, tool lifecycle, approvals, diff, tests, tokens/cost, duration и
  normalized failure;
- заранее выбрать одну primary thesis: integrated tool integrity + attribution
  + trace;
- зафиксировать native Proteus, stock Pi и один конкретный Pi+sandbox profile;
- определить заранее метрики, acceptance threshold, exclusion rules и fixed
  marginal effort budget для Proteus и Pi comparator.

Если record снова распадается на несвязанные session/event/request форматы,
feature work прекращается до решения data cluster.

### Неделя 2: One-Command Comparative Runner

- запускать corpus без ручной склейки логов;
- записывать 100% attempted runs; parse failure, unsupported case и crash
  становятся failed/inconclusive rows и не исчезают из denominator;
- не изменять core ради особенностей одного backend;
- проверить separately packaged out-of-tree plugin, зависящий только от
  `proteus-contracts`, без core changes;
- для module swap заранее определить метрику и promotion/kill threshold, а не
  считать любое различие результатом.

### Неделя 3: Safety И Control Plane

- 20 adversarial integrity cases Proteus, включая child, path/symlink,
  cancellation и approval races, плюс held-out/mutated варианты;
- ноль host-observed escapes в заявленном threat model и denied actions,
  дошедших до spawn;
- на matched normal tasks медианное wall-clock time-to-passing-result не хуже
  pinned Pi+sandbox более чем на заранее выбранные 20%; provider backoff
  отмечается отдельно, denial fast-path не входит в эту метрику;
- 10 web runs с approve/deny/cancel/reconnect/restart-resume без потерянных или
  неверно атрибутированных events; restart-resume относится к root session,
  потому что collaboration children пока не restart-durable.

### Неделя 4: Реальный Dogfood И Решение

- взять десять последовательных подходящих реальных задач по заранее
  объявленным inclusion/exclusion rules, не выбирая их после результата;
- после равного периода familiarization randomized/crossover order сравнивает
  оба backends; tests и artifacts по возможности оцениваются blind;
- model-dependent quality cases повторяются не менее трёх раз на task/config;
  численные thresholds считаются promotion heuristics, а не статистическим
  доказательством;
- предпочтение daily driver записывается как вторичная UX-метрика, а не как
  основной outcome;
- считается доля времени на сам harness, а не на пользовательские задачи;
- выпускается continue/pivot/freeze report.

## Continue / Pivot / Freeze Criteria

Standalone Proteus продолжается только при прохождении всех hard gates:

1. **Primary thesis:** pre-registered integrity/attribution/trace suite проходит
   без host-observed escape в заявленном threat model. Это обязательный
   regression gate, а не доказательство общей security.
2. **Comparative evidence:** one-command runner записывает 100% attempted runs
   минимум для 12 cases, включая crashes/inconclusive, и сравнивает tests,
   artifacts/diff, cost/tokens, tools, approvals, duration и failure. Quality
   cases повторяются не менее трёх раз.
3. **Fair marginal comparison:** Proteus и Pi comparator получают одинаковый
   заранее фиксированный budget до трёх инженерных дней на primary acceptance
   test. После этого Proteus должен либо единственный пройти test, либо дать
   заранее определённый material gain при одинаковых allowances. Это scoped
   spike с фиксированным done condition, а не утверждение, что возможность
   вообще невозможно построить в Pi.
4. **Eval utility:** runner ловит минимум одну реальную или held-out regression,
   выбранную независимо и до реализации соответствующей проверки. Seeded case,
   написанный вместе с runner, подтверждает plumbing, но не закрывает gate.
5. **Maintenance:** работа над harness, adapters, ABI, документацией,
   self-debug и экспериментом занимает не более 25% всех инженерных часов
   30-дневного окна. Denominator фиксируется заранее и не исключает неудобные
   инфраструктурные часы.

Следующие gates условны: claim можно сохранить в позиционировании только если
пройден соответствующий gate. Они не могут компенсировать провал primary
thesis:

- **slots:** отдельно packaged out-of-tree module зависит только от
  `proteus-contracts`, подключается без core changes и проходит заранее
  определённый promotion threshold против baseline;
- **subagents:** не менее 20% медианного time-to-passing-result либо 15
  процентных пунктов success-rate gain при token overhead не выше 30%; это
  заранее выбранные decision heuristics, не статистические нормы;
- **web/control plane:** 10 lifecycle smoke runs проходят без потерянных или
  неверно атрибутированных events, а primary acceptance test требует именно
  наблюдаемого web control, не просто иной renderer;
- **daily-driver UX:** после equal familiarization предпочтение Proteus в
  crossover tasks учитывается только как вторичная UX-метрика.

Pivot к Pi extension или тонкому RPC control plane выполняется, если:

- тот же differentiator реализуется поверх Pi менее чем за треть времени/кода;
- safety-преимущество исчезает при сравнении с Pi+sandbox;
- более половины работы снова уходит на providers, auth, UI, ABI или parity;
- Pi выполняет не менее 80% реальной работы, а Proteus в основном чинит себя.

Standalone runtime замораживается, если к 2026-08-12 нет one-command
comparative report, ни один module experiment не прошёл data-based
promotion/kill decision, а основной аргумент остаётся «Rust» или «моя
архитектура».

Freeze не означает удалить репозиторий. Нормальный outcome — tagged research
archive, сохранение или отдельная публикация уже выделенного
`proteus-process-host`, либо тонкий policy/eval control plane над
Pi/Codex/OpenCode.

## Были Ли Месяцы Потрачены Зря

Часть исходной продуктовой гипотезы действительно обесценилась: generic
extensible harness уже существует и лучше упакован. Это нельзя исправить
списком отличий.

Но результат месяцев не сводится к абстрактному «научился»:

- создан работающий policy/approval/sandbox path;
- выделен reusable process host;
- собраны correlated events, app-server и два клиента;
- реализован и протестирован сложный subagent/worktree lifecycle;
- накоплены конкретные failure cases по contracts, compaction, sessions,
  approvals и provider shaping.

Эти активы не обязывают продолжать standalone runtime. Их ценность в том, что
они позволяют за 30 дней честно проверить более узкую гипотезу, выделить
пригодные части или вовремя остановиться. Правильное использование уже
вложенных месяцев — не защищать прошлое, а резко ограничить следующий риск.

## Документационные Изменения После Решения

Если эксперимент подтверждает pivot:

- `README.md` начинает с policy/eval purpose, а не generic modularity;
- `docs/product/spec.md` заменяет «без форка CLI» на controlled experiment invariant;
- `docs/product/scope.md` называет Proteus policy-first dogfood/experiment rig;
- `docs/product/roadmap.md` получает датированное superseding decision, сохраняя
  старый журнал;
- `docs/architecture/architecture.md` остаётся factual reference и уточняет границу
  external runtimes;
- любой новый pack получает baseline, hypothesis, metrics и review date.

Если эксперимент не проходит gates, authoritative docs не переписываются под
несостоявшийся pivot: проект фиксируется как research archive либо тонкий
integration layer.
