# Координация Нескольких Proteus

Статус: typed agent-control DTO v1 и локальный stdio messaging slice
реализованы 2026-08-25; отделённое durable дерево, attach и remote transport
ещё не реализованы и не стабилизированы.

## Коротко

Subagent в целевой архитектуре — не облегчённый внутренний loop и не component
worker. Это другой одновременно работающий экземпляр Proteus:

```text
root Proteus
    |
    +-- Proteus: research
    +-- Proteus: coding
    `-- Proteus: review
```

Слово `subagent` описывает отношение к root session, а не другой тип
программы. Каждый участник остаётся полным Proteus со своим `AppConfig`,
`AssemblyPlan`, `AgentRuntime`, session/journal, model, tools и policy.

На первом production-срезе root Proteus является координатором: хранит дерево
участников, маршрутизирует сообщения и владеет lifecycle. Прямой mesh, где
каждый процесс самостоятельно соединяется с каждым, не нужен для первого
контракта.

## Кто Чем Владеет

В целевом durable control plane root coordinator владеет:

- устойчивой записью участника и связью parent/child;
- role/profile, адресом подключения и session ownership;
- `spawn`, `send`, `follow-up`, `list`, `wait` и `interrupt`;
- bounded concurrency, mailbox, retention и cleanup;
- маршрутизацией событий, результатов, approval и user-input запросов.

Peer Proteus владеет:

- собственным runtime и полным agent loop;
- своей conversation history и canonical journal;
- своим config snapshot и `AssemblyPlan`;
- своим model/tool/policy authority;
- terminal outcome текущего turn-а.

Сообщение не объединяет права двух экземпляров. Root может передать запрос или
перенаправить approval пользователю, но peer исполняет tool только через свой
`ToolRegistry -> ApprovalPolicy -> ToolSafety` путь. Роль, transport или адрес
не дают дополнительных filesystem/network/process прав.

## Это Не Component Worker

Component Runtime исполняет реализацию конкретного slot под authority host-а.
Peer Proteus сам является host/runtime и может иметь собственные components.
Поэтому agent-to-agent lifecycle нельзя притворять обычным
`slot/module_id` export-ом или пропускать через broad `host.*` callbacks.

Для subagents нужен отдельный типизированный process contract управления
агентами. Он может использовать общие transport/lifecycle primitives, но не
является вторым путём Component Runtime и не возвращает native ABI.

Независимые оси остаются раздельными:

- model-facing surface: `task`, `collaboration`, `none`;
- execution/connection transport: локальный stdio, подключённый app-server,
  будущий remote transport;
- agent profile: model, prompt, workflow, tools и policy;
- workspace policy: общий `cwd`, отдельный worktree или будущая remote
  workspace;
- root-owned semantic record: identity, parent edge, mailbox и terminal state.

## Что Уже Реализовано

Текущий `process` runner уже доказывает важную часть модели:

- использует typed `AgentAddress`, `AgentControlMessage`, lifecycle snapshots
  и operation receipts из `proteus-contracts`;
- запускает отдельный `proteus server stdio` с named child config;
- держит несколько процессов одновременно;
- поддерживает `spawn`, `wait`, `interrupt`, cancel и process pool;
- предоставляет те же `send_message` и `followup_task`, что и `sequential`;
- доставляет bounded адресные сообщения через stdio на ближайшую model/tool
  boundary, сохраняя FIFO одного mailbox;
- пересылает события, approvals и user input между двумя Proteus;
- сохраняет resume, пока жив соответствующий child process.

Real-process boundary test одновременно запускает два полных Proteus,
передаёт им разные payload, проверяет отсутствие cross-delivery, адресный
cancel и изоляцию падения соседнего process. Успешная terminal-гонка не теряет
принятое сообщение: stdio adapter продолжает ту же логическую generation новым
peer turn. Явный cancel синхронно закрывает mailbox цели, отклоняет поздние
сообщения и перед возвратом ждёт уже начатую transport delivery; после
успешного `cancel` новый envelope или continuation уже не достигает peer.

Это ещё не полный целевой control plane:

- semantic agent record пока связан с памятью root process и живым child;
- нельзя подключить уже работающий Proteus по адресу;
- нет durable agent tree и восстановления связи после restart;
- model-facing facade сейчас root-owned: peer-origin message в sibling не
  является прямым вызовом. Root получает результат через `wait_agent` и
  адресно пересылает его следующему участнику; direct peer mesh не добавлен;
- `sequential` остаётся полезным текущим backend/test baseline, но не задаёт
  целевую сущность subagent-а.

Один mailbox ограничен 32 сообщениями, 64 000 байт суммарно и 16 000 байт на
сообщение. В contract v1 source всегда равен `/root`, а target обязан точно
совпадать с адресом, привязанным root control plane к handle; подменённые
source/target отклоняются до enqueue. `AgentControlMessage` хранит эти поля
отдельно от transport handle, а text projection добавляет source attribution
в model-visible user message. Authority при доставке не объединяется: peer
продолжает исполнять tools только через собственные registry, policy и safety.

## Порядок Реализации

1. ✅ Определить typed agent-control DTO и exact lifecycle/failure semantics
   для `spawn/send/follow-up/list/wait/interrupt`.
2. ✅ Дать текущему stdio process path адресные mailbox и follow-up, проверив
   одновременную работу минимум двух полных Proteus.
3. Отделить root-owned agent record/tree от конкретного process connection,
   process pool и transport.
4. Добавить authenticated attach к уже работающему local app-server без
   изменения agent semantics.
5. Только затем решать persistence/reconnect, remote transport и нужен ли
   прямой peer mesh.

Каждый срез должен проверять addressable cancel, bounded queues, crash
изоляцию, session ownership, terminal journal semantics и отсутствие расширения
authority.

## AssemblyPlan И Живая Карта

Сейчас `AssemblyPlan` фиксирует только выбранный subagent runner и
model-facing surface; role config остаётся opaque и не попадает в безопасную
JSON projection. В будущем план может получить безопасное summary разрешённых
role profiles и connection backend, но он не должен перечислять конкретные
запущенные peers: они появляются и исчезают во время работы.

Живые экземпляры, их адреса, parent edges, mailbox и состояния относятся к
session-owned runtime projection. Поэтому будущая карта агентов должна
публиковаться рядом с runtime topology, но не становиться вторым config format.

## Не Входит В Первый Contract

- автоматическое обнаружение чужих Proteus в сети;
- доверие к произвольному endpoint без authentication;
- distributed consensus или общий scheduler;
- неограниченные mailbox/process residency;
- автоматическое объединение histories или прав;
- component-to-component вызовы в обход root coordinator.

История вариантов и upstream-сравнение сохранены в
[research/subagent-architecture-options.md](../research/subagent-architecture-options.md).
