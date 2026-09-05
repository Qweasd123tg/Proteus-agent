# Проверенный Срез Сборки Codex

Baseline: `openai/codex` commit
`67cc3c318dc8b5532db6ade4182b1dc6f3870889`, зафиксирован 2026-09-01.
Этот документ описывает существующее evidence. Граница всего первого
экзамена определяется в [roadmap.md](../product/roadmap.md).

## Ordered Commentary И Final

Срез сохраняет два сообщения
`Message(phase=commentary)` и `Message(phase=final_answer)` отдельно:

- `CanonicalModelResponse.messages` — непустой ordered vector;
- `CanonicalMessage.phase` — typed commentary/final_answer или отсутствие
  классификации;
- OpenAI Responses adapter читает phase и возвращает его в следующий request;
- workflows, compactor, journal и history сохраняют порядок сообщений;
- `coding.codex_loop` берёт последнее непустое assistant message
  как terminal output.

Действующие версии: `workflow/v2`, `compactor/v2`, journal schema v3.

Upstream anchors среза: `codex-rs/protocol/src/models.rs`,
`codex-rs/codex-api/src/sse/responses.rs`,
`codex-rs/core/src/session/turn.rs` в указанном commit.

Локальные [fixture](../../crates/proteus-core/src/adapters/openai/fixtures/codex-multi-message-response.json)
и [test](../../crates/proteus-core/src/adapters/openai/tests.rs) проверяют
Proteus на upstream-shaped response. Они не запускают два полных runtimes
и не являются полным differential harness.

## Проверки

```bash
cargo test -p proteus-contracts canonical_response
cargo test -p proteus-core codex_parity_preserves_ordered_commentary_and_final_messages
cargo test -p coding-workflow codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output
cargo test -p codex-compactor
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-core --test module_swap
```

После изменения применяются общие gates из [testing.md](testing.md).

## Граница Evidence

Этот срез не доказывает совпадения live item lifecycle, retry установленного
SSE stream, полного compaction lifecycle, filesystem/network permissions,
deferred tool discovery и AgentControl semantics.

`AssistantTextDelta` содержит текст без item id и typed phase; app transcript
также не экспортирует `MessagePhase`. Typed classification сейчас сохраняется
в canonical response/history/journal.

Необходимость дальнейших изменений определяется согласованным обычным
сценарием. Этот список не назначает следующую реализацию.
