# Pi Extension Composition: Поправка К Process-Only Плану

- Статус: decision input для
  `docs/architecture/process-module-architecture.md`.
- Дата сверки: 2026-08-07.
- Upstream: <https://github.com/earendil-works/pi>, актуальная ветка `main`.

## Причина Повторной Сверки

Первое сравнение отвечало в основном на вопрос, где исполняется расширение:
in-process TypeScript/dylib или внешний process worker. Это неполная ось.
Transport не определяет, выбирает runtime одну implementation или составляет
несколько независимых behaviors на общей lifecycle boundary.

Проверенные upstream surfaces:

- [Extension API](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md);
- [Extension types](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/types.ts);
- [stateful todo example](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/todo.ts);
- [Pi Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md);
- [custom providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/custom-provider.md).

## Что Pi Делает Иначе

Pi загружает несколько extensions одновременно. Один extension может разделять
state/connection между tools, commands, input/context/tool/provider hooks,
session lifecycle и UI. Transforming handlers образуют load-ordered chains:
следующий handler видит результат предыдущего. Tools/providers могут
регистрироваться после startup, resource paths добавляются при discovery,
extension state сохраняется в session entries или tool result details и
восстанавливается по текущей branch.

Это не contract-level replaceability. Это additive composition и широкая
programmability одного доверенного runtime.

## Реальный Gap Proteus

Целевая форма до этой поправки была только такой:

```text
slot -> one selected process worker
```

Она хорошо выражает замену Search/Model/Workflow backend-а, но плохо выражает
git checkpoint + path guard + input transform + status contribution, которые
должны работать одновременно. Попытка положить всё в Workflow создаёт god
contract; отдельные workers теряют общую live state instance; module-id hooks в
core нарушают equality invariant.

Вторая дыра — state. Per-module data root не знает, к какой session branch
относится запись. После fork/reload/lazy restart внешний SQLite/JSON может
оказаться новее canonical branch и сделать replay недостоверным.

Третья дыра — authority claim. Slot dispatch контролирует `host.*`, но
доверенный executable без sandbox всё ещё имеет прямые OS-права пользователя.
Pi сообщает full-system-access явно; Proteus должен так же отделять
protocol-visible authority от effective OS authority.

## Что Не Нужно Копировать

Pi extension code исполняется с полными системными правами, API привязан к
TypeScript/TUI objects, mutations зависят от load order, а изменённые
`tool_call` arguments не проходят повторную schema validation. Permission gate
является обычным extension, а не обязательным host invariant. Эти свойства не
переносятся в Proteus как есть.

## Принятая Поправка

Process-only transport сохраняется. Contract получает host-defined composition
mode:

```text
select_one   # одна заменяемая implementation
ordered_many # детерминированная цепочка равноправных contributions
```

Protocol v1 переносит composition в strict handshake, но первый Search proof
остаётся `select_one`. Production `ordered_many` contract появляется только
после двух simultaneous use cases, typed chain semantics, revalidation,
failure ordering и branch-aware state design.

Для stateful cutover обязателен host-owned namespaced state/reconstruction
contract. Для полного authority claim обязателен uniform launch/sandbox
profile. Package manager, arbitrary TUI components и hot reload остаются
отдельными product surfaces, а не свойствами process transport.
