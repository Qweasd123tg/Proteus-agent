# ForgeCode Gotchas

Это не “плохо написано”, а места, где поведение важно понимать до копирования архитектуры.

## 1. Persistence не полностью без потерь

Есть риск потерять часть request-shaping состояния при сохранении/восстановлении conversation.

Что важно:

- `ContextRecord` не хранит все поля `Context`
- `response_format` при reload сбрасывается
- conversation с чисто конфигурационным состоянием может не сохраниться так, как ожидается

Смотри:

- `source/crates/forge_repo/src/conversation/conversation_record.rs:821`
- `source/crates/forge_repo/src/conversation/conversation_record.rs:960`
- `source/crates/forge_domain/src/context.rs:431`

Для своего агента это значит: если у тебя есть structured output modes, persistent schemas или режимы ответа, их лучше хранить явно, а не надеяться, что весь `Context` восстановится как есть.

## 2. Attachments — временные

File attachments и directory listings помечаются как `droppable`, а compaction их удаляет.

Смотри:

- `source/crates/forge_domain/src/context.rs:474`
- `source/crates/forge_app/src/compact.rs:153`

Не стоит считать attachment-ы долгосрочной памятью.

## 3. Semantic search может “исчезать”

Если workspace backend/auth/index check ломается, tool availability может превратиться просто в `false`.

Смотри:

- `source/crates/forge_services/src/context_engine.rs:342`
- `source/crates/forge_app/src/tool_registry.rs:250`

То есть пользователь может увидеть не “semantic search backend unhealthy”, а “инструмента нет”.

## 4. Reasoning continuity упрощается

Во время compaction и provider transform reasoning chain сокращается:

- compactor оставляет только последний reasoning block из окна
- provider-specific маппинг может еще сильнее его упростить

Смотри:

- `source/crates/forge_app/src/compact.rs:124`
- `source/crates/forge_app/src/compact.rs:156`
- `source/crates/forge_app/src/dto/anthropic/request.rs:247`

## 5. Request shape часто строится “с запасом”

Например, OpenAI-compatible request начинается с оптимистичного `parallel_tool_calls = true`, а потом правится transformer pipeline.

Смотри:

- `source/crates/forge_app/src/dto/openai/request.rs:403`
- `source/crates/forge_app/src/dto/openai/transformers/pipeline.rs:33`

Это рабочий подход, но он делает reasoning о request shape менее очевидным.

## Что взять себе, а что нет

Стоит взять:

- явный orchestration loop
- раздельные слои памяти
- provider abstraction
- structured context, а не просто массив строк

Стоит переосмыслить:

- насколько lossy может быть persistence
- считать ли attachments временными
- скрывать ли retrieval tool при деградации backend-а
- насколько агрессивно сжимать reasoning history
