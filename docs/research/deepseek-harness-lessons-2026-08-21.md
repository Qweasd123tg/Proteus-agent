# DeepSeek Harness И Proteus: Архитектурное Решение

- Статус: research/decision input, не reference текущей реализации.
- Дата решения: 2026-08-21.
- Sequencing status: исходный вывод `R1 Installed Dogfood next` сохранён как
  исторический, но отменён решением владельца 2026-08-22. P0 получил
  технический `GO`, отдельно подтверждённые P1 transport и P2 broker kernel
  завершены; отдельно подтверждённый P3 atomic cutover завершён 2026-08-23.
  Отдельно подтверждённый P4 topology/journal evidence также завершён
  2026-08-23; текущий contour ведётся в `scope.md` и `roadmap.md`.
- Upstream: [`deepseek-ai/deepseek-harness`](https://github.com/deepseek-ai/deepseek-harness).
- Проверенный upstream-срез: `b150a551b8d465e31e418e1b2eaf5e79bbb7d28e`.
- Исходный подробный разбор:
  [`examples/research/deepseek/deepseek-research-report.md`](../../examples/research/deepseek/deepseek-research-report.md).

## Зачем Нужен Этот Документ

Исходный отчёт описывает DeepSeek Harness как возможную основу нового
модульного агента. Proteus уже существует и имеет собственные проверяемые
инварианты, поэтому прямой перенос рекомендаций из отчёта был бы ошибкой.

Этот документ отвечает на более узкий вопрос:

> какие наблюдения Harness подтверждают текущий курс Proteus, какие выявляют
> конкретный пробел, а какие относятся к другому product/runtime trade-off?

## Короткое Решение На 2026-08-21

Исходное решение не меняло позиционирование Proteus и оставляло следующим R1
Installed Dogfood. Sequencing отменён 2026-08-22, bounded Component Runtime v2
P0 получил технический `GO`, а отдельно подтверждённые P1 transport и P2
broker kernel завершены. P3 atomic cutover и P4 topology/journal evidence
также завершены. Сохраняется содержательный
вывод этого документа: Harness не требует
переносить Cordis, переписывать session model или собирать чужие product
capabilities внутри core.

Harness независимо подтверждает четыре уже выбранных архитектурных решения:

1. consumers зависят от contracts, а не от concrete providers;
2. orchestration, model transport, tools, policy и durable state должны иметь
   отдельные boundaries;
3. model-visible state должен быть реконструируем из canonical journal;
4. lifecycle, cancellation и failure domain принадлежат host runtime, а не
   выбираются реализацией модуля.

Полезные новые входные данные распределяются по существующим этапам:

- Model Contract Migration — streamed identity, retry ownership и запись
  effective request;
- Subagent Contract Migration — различие `followup`, `steer` и model-context
  injection в long-lived agent lifecycle;
- Uniform Worker Trust Policy — отдельная network/SSRF policy и uniform
  resource boundary для всех process components.

## Сверка С Текущим Proteus

| Идея Harness | Состояние Proteus | Решение |
|---|---|---|
| Service definition → provider → consumer | Реализовано как `Core -> Contract -> Component Export Implementation` | Сохранять; не добавлять import concrete implementation в core |
| Replaceable agent loop | `Workflow` является process slot | Проверять swap только по measurable behavior; не создавать hooks вокруг workflow |
| Append-only session event log | Есть canonical journal, cold history и terminal settlement | Сохранять как единственный durable evidence path |
| Реконструируемый model request | Есть `model_request_recorded`, prompt replay и config snapshot | Сохранять prompt replay как automated evidence затронутого model/runtime path |
| Provider-neutral stream/model API | Canonical model существует, adapters пока core-owned | Использовать Harness cases в R2 audit; не добавлять one-off provider builtin |
| Tool registry + schema + policy pipeline | Есть `ToolRegistry -> ApprovalPolicy -> ToolOrchestrator -> Tool::invoke` | Уже current invariant; не создавать bypass для component tools |
| Reversible plugin effects/disposers | Process component имеет host-owned lifecycle и общий failure domain | Cordis/HMR disposer model не нужен без dynamic in-process plugins |
| Parallel tool classifier | Есть explicit parallel eligibility в tool surface | Resource-aware scheduler добавлять только после измеримого concurrency defect/eval evidence |
| Approval как отдельная capability | Реализовано contract-bound | Сохранять одинаковый путь для всех origins |
| Sandbox как отдельная capability | Shell имеет fail-closed sandbox, но uniform worker sandbox отсутствует | Закрывать в R4, не считать process boundary sandbox |
| Scoped long-lived agent/inbox | Root steering и первый session-owned subagent control реализованы частично | Использовать как comparison input для R3, не переносить API дословно |
| Dynamic extension/plugin ecosystem | Намеренно отсутствует | Оставить parked до protocol freeze |
| ACP/SDK/remote agent transports | Не входят в текущую цель | Не добавлять раньше local runtime evidence и protocol freeze |

## Главное Различие Runtime Models

Harness строит composition вокруг доверенных in-process plugins и
dependency-injection context. Proteus строит replaceability вокруг strict
process component contract:

```text
Harness:
plugin activation -> service registry -> reversible in-process effects

Proteus:
config selection -> exact component exports -> host-owned process lifecycle
```

Поэтому из Harness нельзя выводить необходимость вернуть native plugin ABI,
general hooks или динамическую регистрацию неизвестных services. Для Proteus
эквивалент полезного lifecycle-инварианта уже формулируется так:

```text
one configured component = one process + one shared failure domain
authority(invocation) = authority(slot, invocation_context)
```

Компонент может экспортировать несколько `slot/module_id`, но не получает
union их прав. General imports остаются parked; same-process reentrancy теперь
проверяется отдельным bounded Component Runtime v2 P0 без direct dispatch.

## Что Добавить В Evidence, А Не В Архитектуру

### Исторический Installed Evidence Checklist

Этот checklist больше не является gate или sequencing prerequisite. Он сохраняет
полезные automated/optional проверки: недостаточно сохранить assistant
transcript, если changeset затрагивает реконструкцию фактического model request.

В installed session следует проверить:

1. cold `/history` после успешного и terminal-error turn;
2. `prompt replay` с recorded effective system/context/tool surface;
3. `workflow replay` на том же canonical journal;
4. intentional component death и lazy restart без повторного side effect;
5. cancel/steering с корректным `TurnSettled`.

Это расширение evidence path, а не новый session format.

### R2 Model Slot Decision

К существующей matrix нужно применить следующие Harness cases:

- стабильная identity streamed tool call при fragment/delta assembly;
- null/empty continuation fields должны либо иметь canonical semantics, либо
  завершаться validation error;
- adapter выполняет одну transport attempt, а retry owner задаётся явно;
- usage, reasoning/cache/service-tier metadata не теряются при shaping;
- effective request записывается после shaping и до provider side effect;
- provider-hosted tools не обходят canonical event и safety attribution.

Эти пункты не предрешают `model/v1`. Они нужны, чтобы выбор process contract
против documented core boundary опирался на полный lifecycle.

### R3 Subagent Slot Decision

Harness полезен различием трёх операций:

```text
followup -> следующий обычный turn
steer    -> ближайшая step boundary
inject   -> model-visible context без самостоятельного wakeup
```

Proteus не обязан копировать названия или точную queue model. R3 audit должен
однако явно ответить:

- какие сообщения durable;
- что будит idle child;
- где проходит step/turn delivery boundary;
- как cancel конкурирует с queued input;
- что сохраняется после process restart;
- как authority и parent attribution наследуются при resume.

До этого нельзя расширять subagent surface отдельными ad hoc callbacks.

### R4 Uniform Worker Trust Policy

Harness показывает ограничение, которое уже явно учтено Proteus: process или
plugin boundary сам по себе не sandbox. Uniform launch policy должна одинаково
задавать для всех slots:

- filesystem read/write scopes;
- public network grants, private-address deny и redirect revalidation;
- process execution и IPC exposure;
- env/secrets projection;
- CPU, memory, output и wall-clock limits;
- persistent data roots;
- audit и approval для повышения прав.

Network policy должна быть отдельной first-class частью launch authority, а
не набором проверок внутри одного web tool. Нельзя выдавать исключение
reference component или конкретному `module_id`.

## Что Не Делать Сейчас

- не переносить Cordis и не создавать второй plugin lifecycle;
- не возвращать dylib/native ABI ради in-process extensions;
- не вводить arbitrary additive hooks без slot governance;
- не строить marketplace, dynamic install или hot reload до protocol freeze;
- не копировать session tree/fork UI без измеримой/eval потребности;
- не добавлять ACP, SDK или remote worker transport раньше local evidence;
- не создавать resource-aware scheduler без конкретного concurrency defect;
- не расширять число модулей только ради повторения package inventory Harness.

## Порядок Дальнейшей Работы

1. ✅ Bounded Component Runtime v2 P0 завершён с техническим `GO`.
2. ✅ Отдельно подтверждённый P1 duplex transport завершён.
3. ✅ Отдельно подтверждённый P2 broker/wire-v3 kernel завершён.
4. После повторной оценки отдельно решать P3; каждый cutover defect закрывать
   focused protocol/conformance regression-ом.
5. Model и subagent migrations принимать отдельно по parity/governance evidence.
6. Отдельно провести trust-policy design.
7. Вернуться к ecosystem/runtime-composition идеям только после Protocol Freeze
   и доказанной потребности внешних авторов модулей.

## Первичные Источники

- [DeepSeek Harness repository](https://github.com/deepseek-ai/deepseek-harness)
- [Architecture](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/architecture.md)
- [Cordis primer](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/cordis-primer.md)
- [Core subsystem](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/subsystems/core.md)
- [LLM streaming](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/subsystems/llm-streaming.md)
- [Tools subsystem](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/subsystems/tools.md)
- [Session subsystem](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/subsystems/session.md)
- [Persistence subsystem](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/subsystems/persistence.md)
- [Sandbox subsystem](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/docs/subsystems/sandbox.md)
