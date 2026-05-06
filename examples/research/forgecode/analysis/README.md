# ForgeCode Analysis

Это разбор `forgecode` с фокусом на три вещи:

- как проходит один `turn`
- как формируется запрос к модели
- как устроена память: `conversation`, compaction и semantic search

Структура:

- `01-core-overview.md` — короткая карта ядра и основных crates
- `02-turn-pipeline.md` — пошаговый путь `user input -> model request -> tools -> stream back`
- `03-memory.md` — слои памяти, compaction, persistence и их ограничения
- `04-request-shaping.md` — как `Context` превращается в реальный payload для провайдера
- `05-semantic-memory.md` — как работает `sem_search` и почему это отдельный слой памяти
- `06-gotchas.md` — места, которые важно понимать перед копированием архитектуры

Если читать по порядку, лучше идти так:

1. `01-core-overview.md`
2. `02-turn-pipeline.md`
3. `03-memory.md`
4. `04-request-shaping.md`
5. `05-semantic-memory.md`
6. `06-gotchas.md`
