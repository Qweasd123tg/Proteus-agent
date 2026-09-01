# Baseline Codex Parity На 2026-09-01

Статус: активный pinned baseline для работ над `codex` pack. Это не заявление
о полной совместимости Proteus с Codex. Старый полный аудит от 2026-07-14
остаётся историческим snapshot; каждый новый срез нужно заново проверять по
текущему upstream, а не переносить выводы из него автоматически.

## Зафиксированный Upstream

| Источник | Commit | Время и назначение |
| --- | --- | --- |
| `openai/codex` `origin/main` | `67cc3c318dc8b5532db6ade4182b1dc6f3870889` | 2026-09-01 15:34:03 UTC; основной implementation baseline |
| Предыдущий локальный snapshot | `0bbea86a6aae37b1f243676db4248000f04ad111` | 2026-07-08; только для истории, отставал от нового baseline на 2076 commits |

Relevant upstream anchors первого среза:

- `codex-rs/protocol/src/models.rs`: `MessagePhase::{Commentary,
  FinalAnswer}` и ordered `ResponseItem::Message`;
- `codex-rs/codex-api/src/sse/responses.rs`: parser сохраняет phase у
  завершённых output items;
- `codex-rs/core/src/session/turn.rs`: каждый assistant item обрабатывается
  отдельно, а последний завершённый agent message становится terminal output.

Локальная копия upstream не входит в release и не является runtime dependency.
Commit фиксируется в тесте и документации, чтобы следующий аудит мог явно
показать drift.

## Первый Срез: Ordered Commentary И Final

До среза OpenAI adapter складывал reasoning, tools и все output messages в
один `CanonicalMessage`. Поэтому два валидных upstream item-а:

```text
Message(phase=commentary, "Проверяю файлы.")
Message(phase=final_answer, "Готово.")
```

превращались в один синтетический assistant message. Терялись item boundary и
phase, commentary мог попасть в semantic final output, а следующий model
request не мог дословно восстановить upstream-shaped history.

Срез меняет общий canonical contract, потому что несколько ordered model
messages являются provider-neutral фактом ответа, а не особенностью module id:

- `CanonicalModelResponse.messages` хранит непустой ordered vector;
- `CanonicalMessage.phase: Option<MessagePhase>` типизирует `commentary` и
  `final_answer`; отсутствие phase остаётся валидным для других providers;
- OpenAI Responses adapter строго читает role/phase и возвращает phase в
  следующий request;
- workflows, journal, replay, compactor и steering сохраняют все сообщения в
  исходном порядке; app transcript не склеивает их, хотя его public DTO пока
  не экспортирует typed phase;
- `coding.codex_loop` сохраняет commentary в history, но terminal output берёт
  из последнего непустого assistant message, как upstream `last_agent_message`;
- старое wire-поле `CanonicalModelResponse.message` удалено без alias и dual
  reader: проект pre-release, все tracked consumers обновлены атомарно;
- versioned boundaries переключены целиком: `workflow/v2`, `compactor/v2` и
  journal schema v3. Старые версии не получают compatibility reader.

Fixture
`crates/proteus-core/src/adapters/openai/fixtures/codex-multi-message-response.json`
фиксирует минимальный upstream-shaped response. Это первый differential
fixture, а не полный differential harness всего агента.

## Evidence

Focused проверки:

```bash
cargo test -p proteus-contracts canonical_response
cargo test -p proteus-core codex_parity_preserves_ordered_commentary_and_final_messages
cargo test -p coding-workflow codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output
cargo test -p codex-compactor
```

Boundary проверки:

```bash
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-core --test module_swap
```

Полный gate остаётся `cargo test --workspace --no-fail-fast` плюс format,
workspace check и применимые client builds.

## Что Ещё Не Является Parity

Этот срез не закрывает:

1. phase-aware presentation: текущий `AssistantTextDelta` не несёт item id или
   `MessagePhase`, а app transcript DTO не экспортирует phase, поэтому typed
   classification пока остаётся в canonical response/history/journal;
2. retry уже установленного SSE stream и upstream timing обработки
   `output_item.done`/in-flight tools;
3. полный compaction lifecycle и remote compaction capabilities;
4. upstream permission profile для filesystem/network/environment;
5. native deferred tool discovery/history shape;
6. точные AgentControl hierarchy, fork/resume и durable attach semantics.

Следующий связный срез — phase-aware live item lifecycle. Он должен сначала
зафиксировать upstream stream trace и client-visible ожидаемый результат, а
затем решить, достаточно ли расширить общий event contract. Добавлять
provider-specific metadata heuristic в `coding.codex_loop` нельзя.
