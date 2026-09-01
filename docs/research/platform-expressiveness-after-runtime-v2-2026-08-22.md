# Выразительность Платформы После Runtime v2

Дата: 2026-08-22.

Статус: historical research / decision backlog. Заметка была зафиксирована до
P3; atomic cutover завершён 2026-08-23, и действующий contract теперь
Component Runtime v2 / wire v3. Остальной текст сохраняет исходный контекст и
не является разрешением нового public slot или следующей contract migration.
P4 topology/journal evidence отдельно подтверждён и завершён 2026-08-23.

## Зачем Эта Заметка

Proteus задуман прежде всего как личный долгоживущий конструктор агентов, а не
как попытка собрать Pi, Codex, DeepSeek Harness, Hermes, OpenClaw и другие
проекты в один обязательный runtime behavior.

Цель конструктора:

> будущая идея должна по возможности требовать нового component, profile или
> implementation, а не переписывания core.

Эта цель не означает, что Proteus однажды реализует все возможные функции.
Модели, API, UI и способы orchestration будут меняться. Долговечность должна
даваться стабильным способом замены периферии, понятным журналом и маленькой
host-owned семантикой.

## Главный Тезис

После production Runtime v2 платформа должна быть достаточно выразительной для
практической реконструкции разных agent runtimes. Следующая архитектурная идея
не становится изменением Core автоматически.

Желаемый разрез:

```text
boring host kernel
  -> contracts / authority / lifecycle / cancel / journal / replay
  -> profiles and external components
  -> arbitrary internal agent behavior
```

Свобода находится внутри component. Строгость находится на границе с host.
Component может содержать собственный loop, hooks, scheduler, несколько
внутренних агентов или сторонний framework. Proteus не должен понимать эту
внутреннюю архитектуру, пока она не требует новой host-owned semantics.

Это не разрешает generic actor runtime, direct module links, ambient hooks или
особые права по `module_id`.

## Отброшенная Milestone Рамка

Ранняя версия этой заметки связывала завершение Runtime v2 с условным публичным
milestone. Эта рамка больше не используется: архитектурное основание считается
собранным, а дальнейшая проверка идёт через независимые reconstruction
experiments, а не через подгонку platform work под номер выпуска.

## Не Сделать Ли Strict Contracts Узким Горлышком

Риск реален. Contract становится клеткой, если он описывает алгоритм
implementation, дробит agent loop на host-owned микростадии или требует нового
slot для каждой идеи.

Guardrails:

1. Contract задаёт минимальную наблюдаемую semantics, а не внутренний дизайн.
2. Новая идея сначала проверяется внутри существующего component/profile.
3. Model-invoked действие сначала рассматривается как `tool`.
4. Новый `module_id` существующего slot предпочтительнее нового slot.
5. Новый slot нужен только для повторяемой host-owned authority, lifecycle,
   persistence, composition или failure semantics.
6. Для нового slot желательны две независимые реализации; исключение требует
   явного решения владельца и отдельного governance evidence.
7. Нельзя расширять DTO или добавлять hook только ради одного upstream режима.
8. До protocol freeze contract можно ломать атомарно вместо накопления shims.
9. Стоимость простого эксперимента измеряется: если небольшой Pi extension
   превращается в неделю правок core, нужно проверить DX и ширину contract.

Runtime v2 важен именно для этого: крупный component сможет иметь несколько
exports, concurrent invocations и host-routed callbacks, не заставляя host
знать его внутренний dependency graph.

## Возможность Без Нового Slot

Без нового slot можно добавить многое. Без какого-либо contract host-visible
возможность добавлять нельзя.

| Наблюдение | Первый путь | Когда нужен новый contract |
|---|---|---|
| внутренняя стратегия, cache, hooks | private component behavior | host не должен её видеть |
| новое model-invoked действие | existing `tool` contract | tool semantics недостаточно |
| другой loop, context, memory | implementation существующего slot | меняется общий host lifecycle |
| сочетание готовых возможностей | profile/config | host должен владеть новой composition |
| новая панель или projection | client над canonical state | появляется новая runtime command |
| background/event behavior | сначала workflow/component experiment | нужны host scheduler, ownership и journal |

External component не может объявить новый `host.*` method и потребовать его
вызова. Если host обязан понимать такую операцию, она проходит slot governance.

## Expressiveness Test Из Пяти Агентов

После P4, но до попытки расширять platform surface, выполнить design/evidence
matrix для пяти намеренно разных форм:

1. минимальный Pi-like single loop с небольшим tool set;
2. Codex-like coding workflow с shaping, compaction и dynamic tool exposure;
3. plan/execute/review профиль с разными implementations существующих slots;
4. Hermes/OpenClaw-like long-lived assistant с events, memory и background
   work;
5. delegated research agent с concurrent subagents и resumable state.

Для каждой формы ответить:

- собирается ли она только profile-ом и существующими contracts;
- нужна ли новая implementation;
- какая конкретная host-owned semantics отсутствует;
- нужен ли новый contract или достаточно private component design;
- какие parity/eval evidence подтверждают вывод.

Положительный результат — не «Proteus скопировал пять продуктов». Достаточно,
чтобы каждая форма была выразима либо выявляла один точный contract gap без
специальной ветки по upstream или `module_id`.

## Compatible И Inspired Profiles

Upstream reference может привести к трём разным результатам:

- `compatible` — точное повторение доступной upstream semantics, включая stop,
  errors и failure paths; требуется parity evidence;
- `inspired` — самостоятельный режим, который явно заимствует некоторые идеи
  и не обещает parity;
- обычный Proteus profile — использует общие modules без upstream branding.

Creative fallback или «улучшение» нельзя незаметно добавлять в compatible
режим. Улучшенная версия получает отдельное имя/profile/feature flag.

## Hermes И OpenClaw

Hermes и OpenClaw добавлены в research queue как проверка не-coding формы
агента. До начала исследования нужны точные upstream repositories, revision и
материалы владельца проекта: эти имена неоднозначны, угадывать источник нельзя.

Research должен извлекать не inventory функций, а gaps:

- long-running lifecycle и restart;
- events, timers и proactive/background work;
- durable identity и memory;
- messaging/application channels;
- tool/skill composition;
- subagent ownership;
- authority, secrets и network;
- recovery и observable terminal state.

Результат каждого пункта: `existing contract`, `new implementation`, `new
contract` или `parked`. Исследование не является обещанием совместимого
профиля.

## Урок Pi Session Log

Pi показывает полезный user-facing слой над JSONL session tree: resume,
навигацию, branch/fork/clone и export. Это не следует смешивать с повторным
исполнением side effects.

Желаемое разделение Proteus:

```text
view      read-only отображение canonical journal
branch    новый owned continuation от выбранной точки
simulate  orchestration replay на записанных outcomes
rerun     явный live rerun в disposable sandbox/worktree
```

Текущие prompt/workflow replay уже различают provider rerun и side-effect-free
orchestration replay. Будущий branch/rerun design должен отдельно решить:

- canonical identity и ownership новой ветки;
- snapshot config/profile и workspace state;
- политику повторения tool side effects;
- deterministic simulation boundary;
- durable schema и UI projection.

Нельзя автоматически повторять shell/file/network actions из старого journal.

## Условия Возврата К Идеям

Заметка закрывается постепенно, когда:

- после P4 составлена expressiveness matrix пяти форм;
- определена переносимая единица agent profile/bundle;
- внешний component demo подтверждает основной тезис;
- Hermes/OpenClaw research привязан к точным источникам;
- session `view / branch / simulate / rerun` получает отдельное решение;
- каждый выявленный gap либо проходит slot governance, либо явно parked.

До этого пункты остаются research backlog и не меняют активный Runtime v2
critical path.

## Что Эта Заметка Не Разрешает

- считать завершённый P4 разрешением следующей contract migration;
- добавлять новый public slot;
- менять production wire v3 вне отдельной contract migration;
- вводить direct same-process dispatch или общий additive hook bus;
- заявлять upstream compatibility без parity evidence;
- переносить research inventory в reference pack;
- превращать новые архитектурные идеи в искусственный release milestone.
