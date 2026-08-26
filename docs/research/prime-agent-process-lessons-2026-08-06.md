# Prime Agent: Что Важно Для Process-Границы Proteus

Статус: research note, не нормативный contract.

Источник — переданный владельцем отчёт по `PrimeIntellect-ai/prime-agent` на
commit `c5991bc853d27754aed345c13ff4d2e05c40f1f4` и package version `0.7.0`.
Выводы отчёта здесь используются как архитектурный пример; upstream отдельно
не перепроверялся в рамках этого среза.

## Что Пример Подтверждает

Prime Agent полезен не как готовая plugin system, а как доказательство четырёх
границ:

1. process даёт lifecycle/failure isolation, но сам по себе не является
   security sandbox;
2. runtime владеет процессами, sessions, cancellation и durable state, а
   исполняемое поведение обращается к нему через typed host requests;
3. любая начатая операция должна завершаться наблюдаемым terminal state;
4. health check обязан проверять минимальную полезную capability, включая
   spawn/initialize/request/dispose, а не только живой PID или `ping`.

Отчёт также показывает антипример для Proteus: capability gaps между
in-process extension API и daemon bridge приводят к несовместимым режимам и
иногда к молчаливым no-op. Это ровно тот класс дефекта, который создают разные
права у `builtin`, `dylib` и `process` implementations одного slot.

## Что Из Примера Не Следует

- Session worker Prime Agent владеет целым root session tree. Process module
  Proteus — более узкая instance одного slot. Эти понятия нельзя смешивать.
- Process-only modules не требуют переносить Prime daemon, TUI, IPython или
  Continual Harness.
- Один transport не означает один всевластный callback API: host methods
  остаются ограничены contract конкретного slot.
- Worker process не отменяет `ToolRegistry`, policy, approval и `ToolSafety`.

## Влияние На План Proteus

- Оставить process supervision core-owned, а module behavior — внешним.
- Делать bidirectional RPC typed и fail-closed; unsupported method/version
  завершается ошибкой, а не no-op.
- Negotiated transport features не могут менять authority между двумя
  implementations одного slot в одинаковом invocation context.
- В conformance gate добавить capability probe, crash после partial progress,
  non-cooperative cancel и terminal attribution.
- Первым product proof сделать внешний agent worker: реальный model/tool loop
  через разрешённые Workflow host callbacks, без доступа к внутренним объектам
  core.

Нормативное решение и последовательность cutover находятся в
[`process-module-architecture.md`](../architecture/process-module-architecture.md).
