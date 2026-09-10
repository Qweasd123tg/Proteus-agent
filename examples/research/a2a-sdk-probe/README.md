# Проверка Готового A2A SDK

Изолированный кандидатный gate для перехода от собственного обмена сообщениями
AgentControl к готовой реализации A2A. Цель перехода — передать развитие общего
межагентного контракта внешнему стандарту. Размер кода не является критерием
принятия A2A. Это исследовательский executable вне root workspace, а не новый
production backend или совместимый режим Proteus.

## Запуск

```bash
cargo run --locked --quiet --manifest-path examples/research/a2a-sdk-probe/Cargo.toml
```

Команда поднимает SDK server на случайном loopback-порту и обращается к нему
через SDK JSON-RPC client; проверка переподключения использует настоящий SSE.
LLM, credentials и внешний сервис не нужны. При первой сборке Cargo скачивает
зафиксированные зависимости. TLS отключён только у этого локального стенда.

На stdout выводится JSON с требованиями, наблюдениями и индивидуальными
`passed`. Exit code: `0` — все требования выполнены; `1` — кандидат не прошёл
хотя бы одно требование; `2` — стенд не смог завершить измерение. Не скрывайте
код `1` через `|| true`: отрицательный результат здесь является результатом
оценки кандидата. Проверка не закрепляет дефект SDK как желаемое поведение.
Отрицательный результат конкретного SDK не означает отказ от стандарта A2A.

SDK зафиксирован на
[`c7cefa0b4276805efbcfd2ddd3238c22e5f36b8f`](https://github.com/a2aproject/a2a-rs/tree/c7cefa0b4276805efbcfd2ddd3238c22e5f36b8f):
`a2a-lf 0.3.0`, `a2a-client-lf 0.2.3`, `a2a-server-lf 0.4.3`.
Проверяется `DefaultRequestHandler` и `InMemoryTaskStore` без локальных patches
SDK, собственного dispatch wrapper или новых A2A extensions.

## Что Проверяется

| Проверка | Требование |
|---|---|
| `task_result` | Поручение завершается и возвращает свой результат |
| `followup_in_same_context` | Следующее поручение получает новый task ID и сохраняет context ID |
| `input_required_resume` | Ответ на уточнение продолжает ту же задачу после `InputRequired` |
| `message_while_working` | Наш текущий сценарий `send_message`: сообщение доходит до работающего executor |
| `targeted_cancel` | Адресная отмена меняет нужную задачу и сохраняет другую |
| `terminal_task_rejects_message_before_executor` | Терминальная задача отклоняет новое сообщение до исполнения |
| `subscription_reconnect` | Потеря SSE subscriber не останавливает задачу; новая подписка получает snapshot и terminal |

`message_while_working` — требование поведения Proteus, а не утверждение,
что A2A запрещает сообщения активной задаче. Последняя terminal-проверка
опирается на требование `UnsupportedOperationError` в
[Send Message](https://a2a-protocol.org/latest/specification/#311-send-message).

## Результат На Зафиксированной Версии

Измерение 2026-09-11: **5 из 7, exit code 1**. Исходный вывод сохранён в
[`result.json`](result.json). Два препятствия:

1. Повторный `SendMessage` с ID работающей задачи возвращает `-32600`,
   `task execution is already in progress`. Executor не получает сообщение.
   Это ограничение готового handler: его
   [`ExecutionManager::start`](https://github.com/a2aproject/a2a-rs/blob/c7cefa0b4276805efbcfd2ddd3238c22e5f36b8f/a2a-server/src/handler.rs#L92)
   отклоняет второй execution для того же task ID. Готовый handler не заменяет
   наш живой mailbox этим вызовом.
2. Сообщение `late` завершённой задаче получает успешный ответ со старым
   результатом `first`. При этом тело executor уже обработало `late`.
   [`drive_execution`](https://github.com/a2aproject/a2a-rs/blob/c7cefa0b4276805efbcfd2ddd3238c22e5f36b8f/a2a-server/src/handler.rs#L314)
   проверяет terminal snapshot после poll executor stream. Это дефект
   проверенной реализации; он не является желаемой семантикой A2A.

Стенд использует детерминированный **синтетический executor**, не настоящий
Proteus. Уточнение закрывает executor stream в `InputRequired`, а ответ
запускает следующую execution той же задачи. Это не доказательство forwarding
наших живых approvals/user-input responders. Следующее поручение проверяет
только context identity: history и config Proteus SDK не восстанавливает.
Отмена кооперативная и реализована самим fixture executor; проверка не
подтверждает kill process, cancel/delivery race или отсутствие поздних side
effects. Независимость двух задач не равна изоляции двух OS processes.
SSE reconnect проверяется при живом server, без restart и durable storage.

Это узкая проверка пригодности конкретного SDK, не полный A2A conformance
test и не сравнение всех реализаций протокола. При смене revision обновляйте
зависимости, lock, метаданные отчёта и повторяйте gate; не меняйте требования
ради зелёного результата.

## Решения Стандарта И Обязанности Proteus

| A2A определяет | Proteus продолжает определять |
|---|---|
| Messages, tasks, states, artifacts | Запуск процессов и выбор профиля |
| `SendMessage`, `GetTask`, подписки и отмену | Связь внешнего context с нашей session/history |
| Agent Card и описание интерфейсов | Собственные modules, tools, policy и journal |
| Состояния ожидания ввода и авторизации | Как локальные approvals/user-input responders представлены через этот обмен |

Сценарий активного mailbox показывает место адаптации нашего поведения.
Сценарий terminal rejection показывает дефект конкретного готового handler.
Смешивать эти два результата и отклонять сам стандарт по ним нельзя.

В production ничего не заменено. Проверка не утверждает, что авторский
протокол лучше A2A, и не требует перенести каждую внутреннюю особенность
AgentControl в собственное A2A extension. Граница перехода описана в
[subagents.md](../../../docs/architecture/subagents.md#граница-a2a).
Выбор SDK и production-интеграция остаются следующей частью перехода;
полноразмерный runtime-срез этим синтетическим стендом не доказан.
