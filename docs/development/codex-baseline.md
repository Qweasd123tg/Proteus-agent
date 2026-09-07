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

Действующие версии: `workflow/v5`, `compactor/v5`, journal schema v4.

Upstream anchors среза: `codex-rs/protocol/src/models.rs`,
`codex-rs/codex-api/src/sse/responses.rs`,
`codex-rs/core/src/session/turn.rs` в указанном commit.

Локальные [fixture](../../modules/reference/model-pack/src/adapters/openai/fixtures/codex-multi-message-response.json)
и [test](../../modules/reference/model-pack/src/adapters/openai/tests.rs) проверяют
Proteus на upstream-shaped response. Они не запускают два полных runtimes
и не являются полным differential harness.

Сквозной [test](../../modules/reference/process-worker/tests/codex_model_resume.rs)
запускает `coding.codex_loop` в process worker с локальным Responses server
(JSON и SSE), выполняет `read_file`, завершает первый runtime process и
продолжает session в новом. Проверяется фактический следующий HTTP request:
порядок items, multipart text в одном message, phase, encrypted reasoning,
точные function arguments и call/result ids. Journal и workflow replay
проверяются для обоих turns. Это restart после завершённого turn, не recovery
посреди исполнения. Отдельный [test](../../modules/reference/model-pack/src/adapters/openai/round_trip_tests.rs)
проверяет custom-tool input/output через journal. Live модель не вызывается.

## Проверки

### Local Compaction И Project Instructions

Для обычного OpenAI-compatible provider перенесён local путь `core/src/compact.rs`
того же baseline: точные prompt/prefix, текущие instructions/reasoning/cache,
summary без tools и output cap, порог 90% известного raw window и сохранение
последних пользовательских сообщений в пределах 20 000 approximate tokens.
При типизированном переполнении summary request удаляется старейший item с
соответствующей парой call/result; остальные сбои имеют пять повторов с backoff,
а отмена и session budget завершаются сразу. Это поведение самого компонента;
recovery после ошибки обычного workflow model call сюда не входит.

[HTTP/process regression](../../modules/reference/process-worker/tests/codex_compaction.rs)
проверяет фактические запросы, короткий retry после HTTP 400
`context_length_exceeded`, единственное исполнение tool и cold history. Применимый
workflow replay этого сценария пока не проходит: внутренний model exchange
смешивается с exchanges workflow. Полный compaction lifecycle, remote branches
и replay класса ошибки этим срезом не подтверждаются.

`codex_context.project_doc_max_bytes` соответствует общему лимиту 32 768 байт
для цепочки проектных инструкций. [Тесты](../../modules/reference/context-pack/src/codex.rs)
проверяют большой одиночный файл, остаток бюджета вложенного каталога и UTF-8.

### Команды

```bash
cargo test -p proteus-contracts canonical_response
cargo test -p model-pack codex_parity_preserves_ordered_commentary_and_final_messages
cargo test -p model-pack --lib adapters::openai::round_trip_tests
cargo test -p proteus-reference-worker --test codex_model_resume
cargo test -p coding-workflow codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output
cargo test -p codex-compactor
cargo test -p context-pack
cargo test -p proteus-reference-worker --test codex_compaction --test compactor_interop
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-core --test module_swap
```

После изменения применяются общие gates из [testing.md](testing.md).

## Граница Evidence

Этот срез не доказывает совпадения live item lifecycle, retry установленного
SSE stream, полного compaction lifecycle, filesystem/network permissions,
deferred tool discovery и AgentControl semantics.

Item identity и typed phase проходят через `model/v4`, live events и app
transcript. Responses fixture отдаёт added/delta/done/completed, включая
позднюю фазу и multipart текст; regression сверяет live ids/text/offsets
с journal и cold app transcript. Web regression проверяет соседние items
с одинаковым текстом и перекрытие /history с SSE. Это не полный upstream
live item lifecycle: остальные типы output items и failure paths этим срезом
не объявляются эквивалентными.

Необходимость дальнейших изменений определяется согласованным обычным
сценарием. Этот список не назначает следующую реализацию.
