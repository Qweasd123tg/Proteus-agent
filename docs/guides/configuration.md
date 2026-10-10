# Конфигурация

Proteus принимает TOML и JSON. Schema pre-release и strict: неизвестные поля
должны приводить к ошибке, а не игнорироваться.

Полный рабочий пример: [configs/config.toml](../../configs/config.toml).
Минимальный fake-model профиль:
[proteus.example.toml](../../examples/configs/proteus.example.toml).

## Resolution

Без `--config` путь выбирается в таком порядке:

1. `PROTEUS_CONFIG_PATH`;
2. `$PROTEUS_CONFIG_HOME/configs/config.toml`;
3. `$HOME/.config/Proteus-agent/configs/config.toml`;
4. XDG config path, если `HOME` недоступен.

`--config codex` означает named config
`<config-dir>/codex.config.toml`. Явный путь с `/` или extension
используется как путь. Config может быть одним файлом или directory; в
directory файлы `.toml` / `.json` merge-ятся лексикографически, затем применяется
управляемый `config-builder.toml`. Он имеет явный приоритет независимо от имени
других fragments и заменяет целиком `modules`, `module_config`, `active_provider`.

Directory mode предназначен для одного profile, разложенного на fragments.
Каталог `configs/` в репозитории содержит альтернативные named profiles
(`codex`, `glm`, `opencode`) и потому не должен передаваться целиком через
`--config configs`: выбирайте конкретный файл или named config.

```toml
include = "../../configs/proteus.provider.example.toml"
```

`include` принимает строку или array строк. Пути относительны к текущему
config file. Includes merge-ятся слева направо, затем текущий file
перекрывает результат. Objects merge recursively; arrays и scalar values
заменяются целиком. Include cycle — ошибка.

Tracked `codex`/`glm` profiles используют явные fragments:

```text
configs/fragments/openai-proxy.toml  provider launch/credential references
configs/fragments/codex-runtime.toml parent modules/tools и peer lifecycle/routing
configs/fragments/codex-profile.toml strict Codex policy/context overlay
configs/fragments/codex-peer-runtime.toml общий workflow/components/runtime peers
configs/fragments/codex-{explore,coder}-peer.toml prompt/tools/policy конкретного peer
configs/codex-{explore,coder}.config.toml provider/model named child configs
```

Fragment не является profile, module pack или неявным default: он не
загружается без `include`, а итоговый config по-прежнему явно выбирает
provider и каждый behavior slot. Массивы не append-ятся. `components` — map и
merge-ится рекурсивно: например, profile может добавить новый exact export, не
повторяя launch-параметры и остальные exports component-а.

`~`, `$HOME` и `${HOME}` раскрываются в path fields.

## Личные Настройки В Checkout

`configs/` — поставляемые profiles, fragments и prompts для installer и
проверок. Рабочие копии с личными настройками можно хранить в игнорируемом
`.local/configs/`:

```bash
mkdir -p .local
cp -a configs .local/configs
PROTEUS_CONFIG_HOME="$PWD/.local" proteus --config codex-chatgpt
```

Копирование выполняется один раз в отсутствующий `.local/configs/`; затем
редактируются рабочие копии. `PROTEUS_CONFIG_HOME` сохраняет named resolution
и для child profiles. Для постоянного использования того же каталога из CLI
и desktop можно направить `~/.config/Proteus-agent/configs` на абсолютный путь
к `.local/configs/`. Прямая ссылка на tracked `configs/` превращает личные
правки в изменения исходников.

Installer сохраняет существующие named configs, но обновляет managed
`fragments/` и `prompts/`. Личные overrides задаются в named config или его
собственном include. Настройка сворачивания карточек tools хранится клиентом;
устаревшая секция `[web]` не принимается agent config.

## Минимальная Форма

```toml
active_provider = "fake"

[profile]
name = "dev-basic"

[providers.fake]
provider = "fake"
model = "fake-tool-model"
stream = true

[components.reference-model]
command = "proteus-reference-module"

[components.reference-model.exports.model.fake]

[module_config.model.fake]
implementation = "fake"

[modules]
workflow = "coding.single_loop"
context = "simple"
policy = "ask_write"

[components.reference-workflow]
command = "proteus-reference-module"

[components.reference-workflow.exports.workflow."coding.single_loop"]

[components.reference-context]
command = "proteus-reference-module"

[components.reference-context.exports.context.simple]

[components.reference-capabilities]
command = "proteus-reference-module"

[components.reference-capabilities.exports.policy.ask_write]

[tools]
enabled = []

[permissions]
mode = "normal"
```

Reference-модуль должен находиться в `PATH`; `./install.sh` обеспечивает
это для установленного wrapper-а.

`active_provider` необязателен. Если его нет, Core не создаёт model adapter;
`providers` можно опустить или оставить с невыбранными profiles. Пустой или
неизвестный заданный id — ошибка. Обращение workflow к отсутствующей модели
также даёт явную ошибку. Пример сборки без модели:
[`proteus.project-check.example.toml`](../../examples/configs/proteus.project-check.example.toml).
У неё ошибка тестов возвращается с выводом команды; для дополнительного
объяснения можно явно настроить provider и model export.

## Provider Profiles

`active_provider` выбирает key из `[providers]`:

```toml
active_provider = "anthropic"

[providers.anthropic]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
stream = true

[providers.anthropic.reasoning]
effort = "high"
summary = true
budget_tokens = 8192

[module_config.model.anthropic]
implementation = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://api.anthropic.com"
auth = "x-api-key"
api_version = "2023-06-01"

[components.reference-model]
command = "proteus-reference-module"
env_allowlist = ["HOME", "ANTHROPIC_API_KEY"]

[components.reference-model.exports.model.anthropic]
```

Reference-модуль экспортирует следующие model implementations:

- `fake`;
- `openai`;
- `openai_compatible`;
- `openai_codex` (ChatGPT subscription OAuth);
- `anthropic`.

`providers.<name>.provider` — exact id model-export, не встроенный provider enum.
Reference `model-pack` требует `module_config.model.<id>.implementation`
(`fake`, `openai`, `openai_compatible`, `openai_codex`, `anthropic`). Id export произвольный:
два exports могут выбрать одну implementation с разными endpoint/settings.
Core этого ключа не интерпретирует.
Для каждого id нужен явный `[components.<component>.exports.model.<id>]`.
Несколько profiles могут использовать один export, меняя model/stream/reasoning;
разные endpoint, credentials или capabilities требуют разных exports.

`module_config.model.<id>` — opaque provider-owned object. Актуальные варианты
OpenAI/Anthropic shaping лучше брать из tracked configs, а не копировать по
памяти. Credentials можно читать из environment или JSON-файла:

```toml
[module_config.model.openai]
implementation = "openai"
api_key_file = "$HOME/.config/Proteus-agent/secrets/openai.json"
api_key_json_key = "openai_api_key"
base_url_file = "$HOME/.config/Proteus-agent/secrets/openai.json"
base_url_json_key = "base_url"
```

### ChatGPT Subscription Через OAuth

`openai_codex` обращается напрямую к Codex Responses backend с ChatGPT OAuth.
Workflow и исполнение tools остаются в Proteus. Установленный Codex CLI,
OpenCode или отдельный API proxy не требуются.

После `./install.sh`:

```bash
proteus-reference-module auth openai_codex login
proteus-reference-module auth openai_codex status
proteus --config codex-chatgpt
```

Для машины без callback в браузере:

```bash
proteus-reference-module auth openai_codex login --device-auth
```

Device-code login должен быть разрешён в настройках ChatGPT. Обычный вход
слушает `127.0.0.1:1455`, открывает браузер и проверяет PKCE/state. Если порт
занят другим login, команда завершается ошибкой. `--no-browser` печатает ссылку
без автоматического открытия. Вход ограничен 15 минутами, Ctrl+C отменяет его.

Credentials хранятся только у provider-а в
`$HOME/.config/Proteus-agent/secrets/chatgpt.json`. `login`, `status` и `logout`
принимают `--auth-file /absolute/path/chatgpt.json`; тот же путь необходимо
задать в `module_config.model.<id>.auth_file` у всех нужных profiles. Путь к
файлу можно настроить независимо от каталогов установки. Файлы Codex/OpenCode
не читаются и не импортируются. На Unix файл записывается атомарно с mode 0600;
отдельный OS lock сериализует login/logout/refresh между процессами. Refresh
token автоматически обновляется вместе с access token. `status` не раскрывает
токены и не запрашивает сетевой остаток allowance; `logout` удаляет локальную
сессию Proteus, не отзывает все сессии ChatGPT.

```bash
proteus-reference-module auth openai_codex logout
```

Готовый `codex-chatgpt` использует `gpt-5.6-luna` и собственные
`codex-chatgpt-explore`/`codex-chatgpt-coder`: peers также обращаются через
подписку. Модель при запуске задаётся в `providers.chatgpt.model`; доступные
модели и лимиты определяются аккаунтом. Фрагмент `fragments/openai-chatgpt.toml` задаёт explicit
model export, capabilities и консервативный порог контекста 200000 tokens.

В root-профиле `codex-chatgpt` включён самостоятельный
[DCP component](../../modules/reference/dcp/README.md): `hook.dcp`, model tool
`compress` и пользовательская команда `/dcp`. Для запуска нужен Node.js 22+,
собранный модуль и executable `proteus-dcp` в `PATH`; portable и `install.sh`
не устанавливают его автоматически. После сборки в корне checkout можно
подключить entry:

```bash
mkdir -p "$HOME/.local/bin"
ln -s "$PWD/modules/reference/dcp/dist/worker.js" "$HOME/.local/bin/proteus-dcp"
```

Не заменяйте существующий launcher без проверки его назначения. При переносе
checkout обновите ссылку; при переносе модуля сохраняйте его dependencies.
Сборки, включающие `codex-chatgpt` через `include`, наследуют DCP. Самостоятельные
`codex`, `codex-chatgpt-explore` и `codex-chatgpt-coder` его не включают.
Workflow, model и обычный `codex` compactor не заменены: DCP меняет только
outgoing view, а параметры hook/tool exports в профиле одинаковы.

Экспериментальный `context-search-chatgpt` включает этот же profile и меняет
только context selection на `repo_aware`. Запуск:

```bash
proteus --config context-search-chatgpt
```

Он подгружает project instructions, skills, environment и до 8 результатов
поиска по словам текущей задачи через указанный `context_provider/rg_search`. Общий бюджет
контекста — 60000 bytes; model limits, capabilities, effort, workflow, tools
и конфигурация peers наследуются без изменения. Это отдельная сборка для
экспериментов, а не заявление о Codex parity: `repo_aware` также добавляет task
chunk и иначе оформляет project instructions. Поиск может добавить шум и
увеличить первый запрос. Он не видит историю прочитанных файлов и не удаляет
из контекста уже известные модели фрагменты. Выбор обычного `codex-chatgpt`
возвращает `codex_context` той же root-сборки с DCP.

`openai_codex` сам запрашивает `GET /backend-api/codex/models` с ChatGPT OAuth.
Интерфейс приложения показывает все возвращённые модели, включая entries с отметкой «скрытая»,
и только их `supported_reasoning_levels`, включая новые строковые значения.
Успешный каталог кэшируется в модуле на 5 минут; следующая загрузка настроек
после expiry запрашивает его снова. Параллельные загрузки объединяются.
Каталог не расходует inference tokens. При ошибке интерфейс показывает причину,
список не подменяется статическим или API-каталогом. Ошибка не мешает загрузке
остального `/config`; текущая модель остаётся видна. Доступность в каталоге не
означает гарантию квоты на каждый последующий запрос.

Сетевые ошибки каталога и квоты сохраняют первопричину транспорта без URL,
headers и тела ответа. Например, `proxy authorization required` (HTTP 407)
относится к авторизации прокси, а не к ChatGPT OAuth. Эти запросы используют
окружение model component: проверьте `components.<id>.env` и `env_allowlist`.
Личные `HTTPS_PROXY` / `https_proxy` overrides перекрывают окружение родителя;
для прямого подключения удалите overrides и не передавайте proxy-переменные
из родительского процесса.

Остаток подписки доступен через `GET /model/quota` и самостоятельную панель
«Лимиты модели». `openai_codex` читает `https://chatgpt.com/backend-api/wham/usage`
с текущей авторизацией Proteus: основную группу, дополнительные группы, реальные
длительности окон, время сброса и credits, если они предоставлены. Успешный
snapshot кэшируется 30 секунд с объединением параллельных запросов; общий deadline
30 секунд. После expiry ошибка не подменяется старым остатком. После 401 допускается
один refresh; 429 возвращается без повторов. Чтение не запускает inference.

Provider config:

```toml
[providers.chatgpt]
provider = "openai_codex"
model = "gpt-5.6-luna"
stream = true

[module_config.model.openai_codex]
implementation = "openai_codex"
auth_file = "$HOME/.config/Proteus-agent/secrets/chatgpt.json"

[components.reference-model]
command = "proteus-reference-module"
env_allowlist = ["HOME"]

[components.reference-model.exports.model.openai_codex]
```

По умолчанию используются `https://auth.openai.com` и
`https://chatgpt.com/backend-api/codex`. Явные `oauth_issuer` и `base_url` нужны
для тестовых/настроенных endpoints: допустимы HTTPS или loopback HTTP, без
credentials, query и fragment. `quota_url` отдельно задаёт полный endpoint квоты
с теми же ограничениями; по умолчанию это указанный выше `/wham/usage`, независимо
от `base_url`. Для локальных fixtures его задают явно. Provider не следует HTTP redirects.

Подписочный transport всегда SSE с `store=false`; `stream=false`, включая
внутренний complete, собирает ответ из одного SSE request без промежуточных
events. Поле `max_output_tokens` не отправляется, как в выбранном OpenCode;
оно не является provider-enforced output cap этого режима. Общие deadline и
cancellation продолжают действовать. API-key settings,
`stream_error_fallback=true` и `prompt_cache_retention` отклоняются.
HTTP 401 допускает один refresh и повтор запроса; 429 возвращает ошибку лимита
без повторов и без переключения на платный API. Расход идёт по подписочному
доступу/кредитам аккаунта, а не исчезает. Provenance и точные отличия:
[model-pack/UPSTREAM.md](../../modules/reference/model-pack/UPSTREAM.md).

### OpenAI Responses Transport

OpenAI adapter по умолчанию использует согласованную HTTP-версию `reqwest`.
Если OpenAI-compatible proxy некорректно обслуживает Responses API через
HTTP/2, задайте `http1_only = true` в `module_config.model.<id>`. Это transport
compatibility switch конкретного provider profile, а не fallback workflow или
исключение для module id.

В OpenAI и OpenAI-compatible adapter `request_max_retries` задаёт число
HTTP-повторов после первой попытки: по умолчанию 4, `0` отключает повторы,
значения выше 100 ограничиваются 100, как в выбранном Codex. Ключ находится
в `module_config.model.<id>` и принимает только неотрицательное целое число.
Повторяются транспортные ошибки отправки и HTTP 5xx; HTTP 4xx, включая 429,
возвращаются сразу. Задержки начинаются с 200 мс и растут вдвое со случайным
множителем от 0,9 до 1,1. После исчерпания попыток сохраняется последняя ошибка
провайдера. Общий model deadline и cancellation охватывают запросы и задержки,
а не назначаются заново каждой попытке.

Повторы заканчиваются после успешных HTTP-заголовков: ошибки чтения/разбора
JSON body и уже открытого SSE stream не запускают эту политику повторно.
Диагностический `stream_error_fallback` остаётся отдельной явной настройкой;
tracked Codex profile её не включает.

Для SSE OpenAI и OpenAI-compatible adapter принимает
`module_config.model.<id>.stream_idle_timeout_ms`: неотрицательное целое число,
по умолчанию `300000` (5 минут), как в закреплённом Codex. Это ожидание следующего
целого SSE-события после успешных HTTP-заголовков. Полученное событие, даже
игнорируемое adapter-ом, запускает новый интервал; отдельные байты, незаконченный
event и комментарии keep-alive его не продлевают. Время обработки события и
ожидание downstream consumer не входят в этот интервал. `0` задаёт нулевое
ожидание, а не отключает таймер. Non-stream JSON и ожидание HTTP-заголовков
этой настройкой не ограничиваются. Общий model deadline и cancellation
по-прежнему могут остановить запрос раньше.

При idle timeout adapter возвращает `StreamDisconnected` с сообщением
`idle timeout waiting for SSE` и закрывает поток. Это не HTTP retry и не
переход к non-stream запросу: дальнейшее восстановление выбирает workflow.

У `coding.codex_loop` есть отдельный `stream_max_retries` в
`module_config.workflow."coding.codex_loop"`.
По умолчанию это 5 повторов после первой попытки; `0` отключает их, значения
выше 100 ограничиваются 100. Принимается только неотрицательное целое число.
Workflow повторяет `ModelFailureKind::StreamDisconnected` и `Retryable`.
Первая причина обозначает обрыв установленного SSE, idle timeout или EOF без
terminal event. Вторая — временный сбой провайдера, включая retryable
`response.failed` и HTTP 500 после исчерпания внутренних HTTP attempts. Каждый
повтор получает завершённые assistant messages и прежние tool results;
checkpoint сохраняет их до backoff. Задержка начинается с 200 мс, удваивается
со случайным множителем 0,9–1,1 и прерывается отменой. Если `Retryable` содержит
`retry_delay_ms`, используется указанная провайдером задержка. Успешный model response
завершает этот бюджет; следующий model round получает новый. Завершённый item
оборванного потока не обнуляет счётчик. `Other`, ошибки canonical данных,
отмена и общий model deadline не запускают этот retry. Каждый вызов модели
имеет свой model deadline, а общий workflow deadline охватывает все попытки.

Environment читается внутри модуля: нужные переменные (`HOME`, API key,
proxy variables) явно перечисляются в `env_allowlist` component. Это относится
и к `$HOME` в путях JSON secrets. Core не читает credential и не знает схему
настроек провайдера. Reference modules разрешают ключ при первом запросе;
`doctor` проверяет selection, handshake и descriptor, но не доступность ключа
и не соединение с API. Не храните secret literal в tracked config.

Для implementations без discovery варианты reasoning задаются
`providers.<name>.reasoning_efforts` явно. Настроенный default effort остаётся
доступен в меню после отключения reasoning или выбора другого уровня.
Если model export поддерживает
`catalog`, меню использует его модели и effort выбранной модели; список
конфига не дополняет live catalog. Core не выводит effort из имени модели
или endpoint. `/model` и stdio `set_model` возвращают выбранную модель вместе
с актуальным `config`; приложение применяет модель и effort одновременно. При смене
модели поддерживаемый effort сохраняется, иначе выбирается default из каталога.
Неизвестные модель или effort отклоняются. Явный effort `none` сохраняется в
model request как `none`, отключая summary/budget; он показывается в live меню
только если входит в supported efforts.
Anthropic adapter передаёт это отключение отсутствием `thinking` и
`output_config.effort`, сохраняя прежнюю семантику своего endpoint.

`proteus init codex` создаёт top-level `config.toml`, parent/peer fragments,
prompts и named child configs `codex-explore.config.toml` /
`codex-coder.config.toml`. Provider example явно встраивается как в parent,
так и в оба child config; локальный OpenAI proxy из tracked
`codex.config.toml` туда не протекает. Установочные named configs, напротив,
сами выбирают OpenAI-compatible provider и `gpt-5.6-luna`.
В generated runtime роли ссылаются на абсолютные пути созданных рядом
child profiles; запуск parent по явному пути не подхватывает одноимённый
профиль из глобального config home.

## Выбор Behavior Modules

`[modules]` имеет пять optional selection keys (ordered hooks задаются отдельно):

```toml
[modules]
workflow = "coding.single_loop"
context = "repo_aware"
policy = "ask_write"
compactor = "codex"
tool_exposure = "codex_dynamic"
```

Каждый выбранный id должен иметь exact component export. Model выбирается
через provider profile и потому не находится в `[modules]`; root-owned
`AgentControl` настраивается отдельно в `[agent_control]` и behavior slot-ом
не является.

Поле можно опустить. Отсутствие означает structural host behavior, а не
автоматический выбор какого-либо reference module. Для обычных behavior slots
специальных ids `none`, `default`, `process`, `text` и `all_visible` нет;
отсутствующий slot остаётся отсутствующим без compatibility fallback.

## Process Components И Exports

### Настройки Codex Context И Compactor

Reference context implementations `simple`, `repo_aware` и `codex_context`
отвергают неизвестные поля в своём `module_config.context.<id>` при
инициализации модуля. В `repo_aware` и `codex_context` нулевой `memory_limit`
исключает результаты памяти, а `max_search_results = 0` отключает
предварительный поиск.

Автоматическое чтение требует явных `search_provider` / `memory_provider` —
ids объявленных context provider exports. Без id соответствующее чтение
отключено; простого включения tool недостаточно. В packaged profiles поиск
привязан к `rg_search`, а автоматическая память включается только там, где
явно задан `memory_provider`. Provider получает structured query в opaque
metadata, host-owned attribution/settings и cancellation текущей execution.
Context builder не вызывает tools и не получает права записи памяти.

`module_config.context.codex_context.project_doc_max_bytes` по умолчанию равен
`32768`: это общий бюджет исходных байтов проектных инструкций от корня до cwd.
В каждом каталоге выбирается первый непустой `AGENTS.override.md` или `AGENTS.md`;
следующий каталог получает оставшийся бюджет. Один файл может занять все 32 КиБ.
Это отдельная настройка от `max_bytes_per_file` для прочих файлов и
`max_context_bytes` для всего context bundle. Обёртки инструкций добавляются
после чтения; текст сохраняется на границе UTF-8.

В `repo_aware` и `codex_context` загруженные project instructions занимают
`max_context_bytes` первыми; score поиска или памяти не может вытеснить правила.
Остальные chunks выбираются по score и возвращаются в исходном порядке.
Если весь загруженный текст инструкций с обёртками не помещается, context build
завершается явной ошибкой. Уменьшайте исходный `project_doc_max_bytes` либо
увеличивайте общий бюджет, вместо незаметного удаления AGENTS.md.

Local `codex` compactor по умолчанию запускается при достижении 90% известного
сырого окна модели. `module_config.compactor.codex.trigger_tokens` задаёт
абсолютный порог, ограниченный теми же 90%. Если окно неизвестно и явного порога
нет, автоматическое сжатие не запускается. `trigger_fraction` и прежние
environment overrides удалены; старые/неизвестные module-config keys отвергаются.
Summary использует текущую модель и её инструкции/reasoning/cache, без tools
и отдельного лимита 4000 токенов. Срез совместимости и ограничения описаны в
[codex-baseline.md](../development/codex-baseline.md).

Как в pinned Codex, replacement сохраняет только текст исторических user
сообщений: старые изображения удаляются из рабочего контекста. Исходные image
refs и файлы остаются в journal/store. Текущий ввод при pre-turn compaction
добавляется после сжатия и сохраняет свои изображения.

`module_config.compactor.codex.stream_max_retries` задаёт число повторов summary
после первой попытки: по умолчанию `5`, `0` отключает повторы, значения выше
`100` ограничиваются сотней, как в выбранном Codex. Это отдельная настройка
compactor; она не наследует retry budget workflow или HTTP provider. Для
сравнения одной сборки задавайте одинаковый stream budget явно. При переполнении
контекста compactor сокращает историю и сбрасывает счётчик повторов; переполнение
запроса из одного summary prompt, отмена и session budget завершаются сразу.

Без export override compactor наследует общий `runtime.workflow_timeout_ms`:
внутренние model calls и повторные попытки расходуют оставшийся бюджет turn.
`runtime.model_timeout_ms` ограничивает отдельный model call. Protocol deadline
compactor имеет запас 1000 мс для settlement внешнего timeout; общий лимит turn
от этого не увеличивается. `components.<id>.exports.compactor.<module>.timeout_ms`
может явно ограничить всю compaction сильнее. При `workflow_timeout_ms = 0`
compactor требует явный положительный export timeout.

### Объявление Exports

```toml
[components.python-search]
command = "python3"
args = ["examples/modules/search-process/search.py"]
cwd = "."
env_allowlist = ["SEARCH_TOKEN"]
env = { SEARCH_MODE = "local" }
handshake_timeout_ms = 30000
description = "Python ripgrep example"

[components.python-search.exports.tool.python_rg]
timeout_ms = 60000
description = "Python ripgrep export"

[components.python-search.exports.context_provider.python_rg]
timeout_ms = 60000
```

Поля component:

| Key | Значение |
|---|---|
| `command` | executable, обязательно |
| `args` | argv после executable |
| `cwd` | absolute или relative к workspace |
| `env_allowlist` | parent env names, разрешённые child process |
| `env` | scoped literal env; перекрывает allowlist |
| `handshake_timeout_ms` | единый initialize timeout: подготовка, запись и ответ |
| `description` | fallback observability text для exports |
| `exports.<slot>.<module_id>` | непустая exact export map, обязательно |

Поля export:

| Key | Значение |
|---|---|
| `timeout_ms` | invocation timeout override только этого export |
| `description` | observability text только этого export |

Environment процесса очищается. Process host сохраняет минимальный `PATH`,
затем применяет allowlist и literal env. Один component запускается один раз
на canonical workspace; все его exports делят child lifecycle и restart.
Launch config не принимает вложенный module `config`.

Module-owned config:

```toml
[module_config.context.repo_aware]
search_provider = "python_rg"
max_search_results = 50
```

Core требует object, но не интерпретирует его поля. Object соответствующего
export передаётся в component initialize.

Ошибки без compatibility fallback:

- selected id не зарегистрирован;
- duplicate `slot/module_id` внутри или между components;
- unsupported process slot;
- пустые component id/export identity/command;
- component без exports;
- zero timeout;
- unknown component/export field;
- non-object module config;
- handshake component id или exact export-set mismatch.

Один component может экспортировать несколько slots. Это общий lifecycle, а
не объединение authority: callbacks проверяются по активному export. Runtime
v2 допускает concurrent и nested invocation того же component; lineage,
depth, counts и deadlines задаёт host, поэтому transport-cycle validation в
config больше нет. Подробности — в
`process-module-architecture.md`.

Отдельный runnable пример с workflow, context, compactor и capabilities в
одном component — `examples/configs/proteus.one-component.example.toml`.
Это evidence topology, а не новый default: несколько components по-прежнему
нужны, когда владелец хочет разные failure domains.

Model-optional control-flow проверяет отдельный runnable профиль
`examples/configs/proteus.project-check.example.toml`. Он выбирает
`coding.project_check`, `ask_write` и только `git_status`, `list_dir`, `shell`;
запуск test command проходит явный approval.
Fake provider в примере не вызывается на success/unsupported/blocked ветках;
для содержательного объяснения failed test его можно заменить обычным provider
profile. Наличие active provider пока обязательно для всего `AppConfig` и
является одним из зафиксированных результатов probe-а.

## Ordered-Many Modules

`tool` и `context_provider` не имеют keys в `[modules]`. Все объявленные
exports этих slots являются contributions:

```toml
[components.reference-capabilities]
command = "proteus-reference-module"

[components.reference-capabilities.exports.context_provider.skills]

[components.reference-capabilities.exports.tool."reference.tools"]
```

Map iteration даёт детерминированный key order, но не является пользовательской
priority surface. Tool registration фильтруется `tools.enabled`; context
builder запрашивает provider по id через `host.context.provide`, а нужный
порядок providers задаёт его собственный `module_config`.

Пак tools — один такой export, а не отдельная обязательная программа. Например,
`exports.tool.git_tools` и `exports.tool.file_tools` под одним `component`
предоставляют независимые паки в общем процессе. Codex-family fragments
выбирают узкие reference exports `file_tools`, `git_tools`, `shell_tools`,
`plan_tool`, `rust_lsp`, `skill_tool`, `policy_tools` вместо агрегата
`reference.tools`; состав tools и правила их исполнения сохранены. Агрегат
остаётся отдельным допустимым выбором для других сборок.

Tool export получает список specs с bootstrap timeout 30 000 мс. При исполнении
каждого tool process adapter использует его `ToolSpec.timeout_ms` (при отсутствии
— 30 000 мс) с запасом 1000 мс для settlement внешнего tool timeout. Явный
`components.<id>.exports.tool.<module>.timeout_ms` переопределяет protocol budget
всех tools этого export; общий бюджет turn и timeout самого tool продолжают
действовать. Это одинаковый путь для любого component/export, без исключений
по имени tool или реализации.

## Reference Inventory

Удобный dogfood executable `proteus-reference-module` публикует:

```text
model:            fake, openai, openai_compatible, openai_codex, anthropic
workflow:         coding.single_loop, coding.codex_loop,
                  coding.plan_execute_review, coding.project_check
context:          simple, repo_aware, codex_context
context_provider: skills, rg_search, jsonl_memory, sqlite_memory
policy:           allow_all, ask_write, codex_policy, opencode_policy
compactor:        codex
tool_exposure:    codex_dynamic
tool:             reference.tools и узкие selectors, direct_patch, codex_patch,
                  rg_search, jsonl_memory, sqlite_memory
```

Это reference/test inventory, не обязательный пакет. Любой другой executable,
прошедший тот же contract, настраивается тем же способом.

Отдельных selections `modules.search`, `modules.memory` и `modules.patch` нет.
Имена `rg_search`, `jsonl_memory`, `sqlite_memory`, `direct_patch` и `codex_patch`
обозначают implementations общих tool/provider contracts. Имя export и имя tool
различаются: например, `tool/rg_search` предоставляет model-visible `search`.

`tool/rg_search` предоставляет `search`, а `tool/jsonl_memory` либо
`tool/sqlite_memory` — `remember_fact` и `recall_memory`. Эти exports не входят
в `reference.tools`; нужные имена включаются через `tools.enabled`. Подключайте
одну реализацию для каждого имени tool. `/remember` требует enabled
`remember_fact` и проходит тот же policy/approval path.

Для индексируемого `search` есть внешний
[пример на tgrep](../../examples/modules/tgrep-search/README.md) и
[самостоятельный профиль](../../examples/configs/proteus.tgrep-search.example.toml).
Он заменяет tool implementation; context provider отдельно не добавляется.

Одноимённые `context_provider` exports добавляют результаты автоматического
чтения; они не зависят от `tools.enabled` и не вызывают tools. Каждый export
получает отдельный config. Для общего хранилища задайте одинаковый `path`:

```toml
[components.reference-capabilities.exports.tool.sqlite_memory]
[components.reference-capabilities.exports.context_provider.sqlite_memory]

[module_config.tool.sqlite_memory]
path = ".proteus/memory.sqlite"

[module_config.context_provider.sqlite_memory]
path = ".proteus/memory.sqlite"

[module_config.context.repo_aware]
memory_provider = "sqlite_memory"
```

JSONL использует первые substring-совпадения, SQLite — FTS. Алгоритм поиска,
storage и их параметры принадлежат implementation, а не Core.

Codex-family fragments подключают exact tool export
`components.reference-capabilities.exports.tool.codex_patch`; остальные packaged
profiles подключают `tool.direct_patch`. Оба предоставляют `apply_patch`, который
включается через `tools.enabled`; одновременно подключать оба нельзя из-за
дублирующегося имени tool. Отдельной настройки выбора patch slot нет.
Core не знает алгоритма. При смене export согласуйте синтаксис в instructions:
`prompts/codex-default.md` описывает `codex_patch`,
`prompts/direct-patch.md` — `direct_patch`. Installer публикует оба prompt assets.

В рабочих Codex fragments задано
`module_config.tool.codex_patch.reject_self_move = true`: перенос на тот же
нормализованный путь отклоняется до записи любых hunks. Это явное отличие
от pinned Codex. У export настройка по умолчанию выключена; для точного
сравнения используйте `false`. [Граница режима](../../modules/reference/codex-patch/UPSTREAM.md).

## Instructions

```toml
[[instructions]]
kind = "System"
file = "prompts/codex-default.md"
priority = 100

[[instructions]]
kind = "Developer"
text = "Prefer small, verified changes."
priority = 50
```

Entry задаёт ровно одно из `file` и `text`. Relative file path считается
от config file. Runtime превращает entries в ordered canonical
`InstructionBlock` list.

Packaged `codex-default.md` и `opencode-default.md` — настраиваемые prompts
Proteus, не точные копии upstream. Они задают цель, завершение работы и
особенности доступных tools. Runtime parity проверяется отдельно; при
сравнении сборок фиксируйте также instructions и project context.

Общие правила держите в prompt, локальные соглашения — в `AGENTS.md`,
инструкции отдельного workflow — в skill. Описывайте результат и условия
применения; не требуйте читать все справочники или повторять уже успешные
проверки независимо от задачи. Этот подход соответствует рекомендациям
[OpenAI по prompts и skills](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra).
Сокращение текста само по себе не доказывает улучшения качества модели.

### Skills

`skill-pack` ищет `SKILL.md` в подкаталогах `.proteus/skills` корня проекта
и `${PROTEUS_HOME}/skills` (по умолчанию `~/.proteus/skills`). Project skill
заменяет user skill с тем же именем. Frontmatter содержит `name` и
`description`; имя совпадает с именем каталога.
Для user skills процессу capabilities нужны `HOME` и `PROTEUS_HOME` в
`env_allowlist`; поставляемые профили задают их явно.

Отключение отдельных навыков хранится в профиле, а не в настройках приложения:

```toml
[addons]
disabled_skills = ["review", "team.tools:deploy"]
disabled_mcp_servers = ["team.tools:database"]
```

Отключённый skill отсутствует в доступном модели списке и не загружается
инструментом `skill`. Каталог управления сохраняет его описание и состояние.
Core не читает `SKILL.md`: любой `context_provider/v4` может предоставить
typed skill catalog, либо вернуть `null`, если не поддерживает эту возможность.
Одинаковые host-owned `SkillRuntimeSettings` передаются context provider и
`tool/v5`; в reference skill-pack один фильтр используется для списка и загрузки.
`name` ограничен 64 ASCII-символами без `--`, `description` — 1024 символами;
дополнительные поля Agent Skills не предоставляют новых прав tools.

### Локальные Agent Plugins

```toml
[[addons.plugins]]
path = "/home/user/agent-plugins/team-tools"
enabled = true
```

Поддерживается локальный каталог [Agent Plugins 1.0](https://agent-plugins.org/specification):
обязательный `plugin.json` с `$schema`, непосредственные подкаталоги `skills/`
с `SKILL.md` и необязательный `mcp.json`. Относительный `path` отсчитывается
от рабочего каталога агента. Навык пакета вызывается по `plugin-name:skill-name`,
MCP-сервер получает имя `plugin-name:server-name`; навыки проекта и пользователя
сохраняют прежний выбор по имени. Отключение пакета выключает все его contributions.
Индивидуальное отключение MCP работает через `addons.disabled_mcp_servers` для
серверов любого происхождения.

Загрузчик выбирает локальные правила по точному `$schema`; сеть для загрузки схем
не используется. Неизвестные поля корневого manifest сообщаются и игнорируются
по правилам стандарта; прочая ошибка manifest отклоняет пакет. Ошибка `skills/`
или `mcp.json` не блокирует независимые части; неверный skill или MCP entry
пропускается с диагностикой. Неизвестные client extension namespaces игнорируются:
hooks, process exports и панели интерфейса из пакета не исполняются.

MCP импортируется только для `type = "stdio"`; валидные `streamable-http` и
`sse` entries отмечаются как неподдержанные, без смены transport. Пути пакета
не могут разрешаться за пределами его root. `command` остаётся одним executable
token, аргументы передаются отдельно; его `./`-путь разрешается от пакета.
По умолчанию `cwd` — root пакета. `${PLUGIN_ROOT}` и `${PLUGIN_DATA}` разворачиваются
однократно в `args`, `env`, `cwd`, но не в `command`. Persistent data находятся в
`.proteus/plugin-data/<hash canonical package root>` рабочего каталога и сохраняются
при reload, выключении и обновлении пакета. Installer, registry и автоматическое
скачивание пакетов здесь не реализованы.

`GET /addons?session_dir=…` возвращает настройки, provider-owned skill catalogs,
состояния обнаружения MCP и диагностику пакетов. `POST /addons?session_dir=…`
заменяет `{ addons, mcp_servers }`, проверяет новую сборку и сохраняет её тем же
atomic profile path, что Config Builder. Новые exports он не создаёт. Оба endpoint
используют обычные session addressing и token/origin checks. Эти операции
не открывают Turn и не требуют модели. Каталог может запускать настроенный
context provider, но не вызывает skills или tools.

Context provider `skills` передаёт модели только имя, описание и путь.
Тело загружается tool `skill` по имени. Пишите короткое описание конкретного
сценария: например, «создание и проверка миграции БД», а не «любая работа
с данными». Для нескольких workflows оставляйте в `SKILL.md` условия выбора
и ссылки на нужные справочники, чтобы модель читала их по необходимости.

Поставляемый skill [interactive-response](../../configs/skills/interactive-response/SKILL.md)
описывает визуальные ответы `json-render`: карточки, таблицы, графики и вкладки.
Он загружается по обычному skill contract; инструкции рендерера не добавляются
в каждый системный prompt. Установка и ограничения — в
[руководстве desktop](desktop.md#интерактивные-ответы).

## Tools

```toml
[tools]
enabled = [
  "search",
  "read_file",
  "grep",
  "apply_patch",
  "shell",
]
```

Имена должны существовать в одном из sources:

- core facade tools: `request_user_input` / `AskUserQuestion`;
- объявленные component tool exports, включая `apply_patch`, `search`,
  `remember_fact` и `recall_memory`;
- `[[tools.configured]]`;
- discovered `[[tools.mcp_servers]]`;
- provider-hosted tools.

Unknown enabled tool и name collision — ошибка. Tool export сам по
себе не делает tool model-visible; имя должно быть в `enabled`.

### Configured Process Tool

```toml
[[tools.configured]]
name = "lint"
description = "Run the project linter"
safety = "RunsCommands"
supports_parallel_tool_calls = false
timeout_ms = 60000
input_schema = { type = "object", properties = {} }

[tools.configured.executor]
kind = "process"
command = "scripts/lint-tool"
args = []
env_allowlist = []
env = { MODE = "check" }
```

Configured tool — отдельная tool execution surface, не behavior module.
Executor имеет kind `process` или `mcp`; native handlers отсутствуют.

`supports_parallel_tool_calls` по умолчанию `false`: вызов ждёт предыдущие
tools и удерживает следующие до завершения. Значение `true` разрешает совместное
исполнение соседних parallel calls через тот же registry/policy/approval path.
`ReadOnly` сам по себе этого разрешения не даёт.

### MCP

```toml
[[tools.mcp_servers]]
name = "local_echo"
enabled = true
command = "sh"
args = ["examples/mcp/echo_server.sh"]
safety = "RunsCommands"
supports_parallel_tool_calls = false
timeout_ms = 30000
protocol_version = "2025-11-25"
max_response_bytes = 20000
metadata = { scope = "local-smoke-test" }
```

MCP client использует официальный Rust SDK `rmcp` 3.3.0 поверх общего
`ProcessTransport`: SDK отвечает за handshake, typed tool discovery/invocation
и pagination `tools/list`; host сохраняет framing, receive limits и lifecycle
дочернего процесса. `protocol_version` задаёт предпочитаемую версию в
`initialize`; по умолчанию используется `2025-11-25`. Неизвестная SDK версия
в config или ответе сервера отклоняется. Сервер обязан объявить capability
`tools`.

`enabled = false` исключает сервер из запуска и discovery. Необязательный `cwd`
задаёт рабочий каталог процесса; относительный путь отсчитывается от workspace.
Неудачный start/handshake/list изолируется на этом сервере: его tools не
регистрируются, остальные подключения работают, ошибка видна в `/addons`.
Name collision остаётся блокирующей ошибкой сборки. Список `tools` в MCP status
отражает discovery при подготовке epoch, а не непрерывную проверку live health.

Текущий MCP scope — stdio tool discovery/invocation. HTTP, OAuth, resources,
prompts, subscriptions, sampling и elicitation не включаются автоматически
вместе с SDK и не входят в реализованную границу.
Один допущенный вызов tool отправляет один `tools/call` и ожидает завершённый
`CallToolResult`. Автоматические дополнительные раунды SDK (MRTR) и task
continuations не выполняются; иной вид результата завершает generation
с ошибкой.
Для discovered tool параллельный запуск разрешён, если у сервера задано
`supports_parallel_tool_calls = true` или tool объявил MCP
`annotations.readOnlyHint = true`. Без обоих признаков вызов последовательный.
Это правило scheduling не понижает `ToolSafety` и не отменяет approval.
Поддержка multiplexing в SDK сама по себе также не разрешает parallel calls.

Отмена или timeout вызова завершает текущий процесс configured MCP server;
остальные concurrent calls этого сервера получают ошибку. Следующий явный
вызов запускает новый процесс и повторяет handshake, но завершившийся ошибкой
вызов автоматически не переигрывается. Обычный tool result с `isError = true`
становится failed `ToolResult` и не требует restart сервера.

Если `structuredContent` присутствует и не равно `null`, оно сериализуется в JSON
как текст результата для модели и имеет приоритет над `content`; `{}` тоже
является результатом. При отсутствии поля или `null` используется `content`.
Исходное структурированное значение также сохраняется в metadata; `isError`
сохраняет статус ошибки при обоих способах представления.

## Policy И Permissions

```toml
[permissions]
mode = "normal" # plan | normal | auto

[module_config.policy.ask_write]
allow = ["search", "read_file", "grep"]
ask_before = ["apply_patch", "write_file", "shell"]
```

`ModeAwarePolicy` применяется в core поверх выбранной process policy.
Модуль не может обойти `ToolSafety` или approval transport.

`allow_all` полезен только для контролируемых profiles; это ordinary
reference implementation с той же authority.

## Tool Exposure

Codex-family profiles не задают `modules.tool_exposure`: workflow получает
весь набор tools, видимых по текущей policy, без hot set и дополнительных
`proteus_tool_*` посредников. Состав задаётся `tools.enabled` и настроенной
AgentControl surface; approval проверяется при исполнении.

Для собственной сборки можно отдельно включить эвристический selector
`codex_dynamic`. Это алгоритм Proteus, он не воспроизводит upstream Codex
`tool_search`:

```toml
[modules]
tool_exposure = "codex_dynamic"

[components.reference-capabilities]
command = "proteus-reference-module"

[components.reference-capabilities.exports.tool_exposure.codex_dynamic]

[module_config.tool_exposure.codex_dynamic]
max_hot_tools = 16
```

Если selection отсутствует, host передаёт workflow все policy-visible tools.
Это structural behavior, не скрытый module id.

`max_hot_tools` — положительное целое, `always_include` — массив непустых
имён tools (пустой массив очищает поставляемый список). Эти настройки читаются
из config выбранного export, а не из invocation. Неизвестные keys и неверные
типы отвергаются. Configs `ask_write`, `codex_policy` и `opencode_policy`
также отвергают неизвестные поля, включая опечатки в `allow`, `ask_before` и `deny`.

## Subagents

```toml
[agent_control]
surface = "task" # task | collaboration | none
max_depth = 1
cancel_grace_ms = 5000
max_parallel = 8
max_idle_processes = 8

[[agent_control.roles]]
name = "explore"
description = "Read-only codebase explorer."
config = "codex-explore"
parallel_safe = true
max_processes = 4
timeout_ms = 14400000
max_summary_bytes = 8192

[[agent_control.roles]]
name = "coder"
description = "Worktree-isolated coding peer."
config = "codex-coder"
isolation = "worktree"
max_processes = 4
timeout_ms = 14400000
max_summary_bytes = 8192
```

`config` — named config (`<config-dir>/<name>.config.toml`) или явный путь к
конфигу другого полного Proteus. Его provider/model, instructions, workflow,
tools, policy и содержательные ограничения не наследуются от root. Parent role
содержит только имя/описание, config reference и технические process/lifecycle
bounds. Packaged `codex.config.toml` использует `codex-explore` и
`codex-coder`; их tool surfaces и policy находятся в отдельных child profiles.

In-process mini-agent, loop-oriented slot и его inline role schema удалены без
legacy alias, fallback или dual-read. `agent_control` — отдельная top-level
секция: `modules.subagent` и `module_config.subagent.*` являются неизвестной
формой config и завершаются явной ошибкой.

`surface` выбирает model-facing facade:

- `task` — один delegation tool;
- `collaboration` — spawn/list/wait/interrupt и bounded
  `send_message`/`followup_task` через единый process backend;
- `none` — agent-control tools не регистрируются.

Инструменты выбранной facade управляются runtime: topology и редактор
профиля показывают их включёнными и не добавляют их в `tools.enabled`.
Их состав изменяется через `agent_control.surface` и настроенные роли.

Это единственный `none` в schema: enum UI surface, а не module id. Текущий
активный baseline считает process agent отдельным полным Proteus; старый
loop-oriented slot удалён, а process pool и обе facade скрыты за единым
`AgentControlRuntime`. См. [subagents.md](../architecture/subagents.md).

## Runtime, Server И Events

```toml
[runtime]
model_timeout_ms = 10800000
context_timeout_ms = 30000
workflow_timeout_ms = 14400000

[app_server]
approval_timeout_ms = 0

[event_log]
path = ".proteus/events.jsonl"
persist_deltas = false
```

Zero `approval_timeout_ms` означает отсутствие server-side deadline для
ожидания ответа пользователя. Export timeouts задаются в component config
и не заменяют общие runtime limits.

Настройки отображения не входят в agent config. Компактные карточки
инструментов включаются в разделе «Настройки → Чат» и сохраняются в клиенте
для всех его чатов. Секция `[web]` не принимается; автоматического переноса
старой настройки из config нет.

## Ordered Hooks

```toml
[modules]
hooks = ["hook.instructions", "hook.output_budget"]

[components.hooks]
command = "proteus-reference-module"

[components.hooks.exports.hook."hook.instructions"]
timeout_ms = 5000

[components.hooks.exports.hook."hook.output_budget"]
timeout_ms = 5000

[module_config.hook."hook.instructions"]
text = "Объясняй результат по-русски."
placement = "append"

[module_config.hook."hook.output_budget"]
max_bytes = 4096
head_bytes = 1024
```

Порядок массива является порядком выполнения, не сортируется по id или
component. Пустой массив (default) отключает contributions. Blank, duplicate
и неизвестные ids отклоняются до сборки runtime. Не выбранный hook export
остаётся available. Config принадлежит `module_config.hook.<id>`; timeout
отдельного export перекрывает default 5000 ms.

В настройках приложения страница «Агент → Обработчики» включает hooks и
меняет их порядок кнопками. Save других slots сохраняет этот список.
`POST /config/builder` принимает отдельное `hooks: string[]`; отсутствие поля
сохраняет прежний список, пустой массив отключает hooks.

Для собственных JS/TS handlers есть внешний
[`hook-process` модуль и SDK](../../examples/modules/hook-process/README.md).
Node.js 22.18+ загружает entry-файл, `hooks.on` регистрирует canonical события,
а optional `tools` фильтрует точные имена инструментов. Export config содержит
`entry` и необязательный object `settings`. Обёртки помогают переносить узкие
Pi/OpenCode tool handlers (включая изменение args), JSON-stdin PreToolUse
`updatedInput` и Stop scripts Codex/Claude Code. Completion review сохраняет
root turn, имеет общий workflow deadline и лимит 8 продолжений.
Неподдержанные upstream действия отклоняются явно; это адаптер переноса,
не загрузчик чужих plugins. Модуль использует тот же `hook/v4`, без отдельной
registration или authority surface в Core.

### DCP Context Pruning

[`proteus.dcp.example.toml`](../../examples/configs/proteus.dcp.example.toml)
подключает самостоятельный Node component с `hook.dcp` и tool export `dcp.tools`.
Сборка, настройки, происхождение и лицензия описаны в
[`modules/reference/dcp/README.md`](../../modules/reference/dcp/README.md).
Выбор включает `hook.dcp` в `modules.hooks` и `compress`, `dcp` в `tools.enabled`;
opaque config двух exports должен совпадать. Пример не устанавливает executable
автоматически: нужен абсолютный путь к подготовленному `dist/worker.js`.

DCP заменяет только outgoing model context, не cold history; выбранный
`compactor` остаётся независимым. `/dcp stats`, `/dcp context` и
`/dcp decompress [NUMBER]` принадлежат тому же component; управление не
предоставляется модели. Это механизм из DCP 3.2.0, не вся оболочка OpenCode
с её commands/TUI/RPC и не объявление parity экспериментального профиля.

### Prompt-команды

Profile может добавить текстовые команды в общий каталог CLI/приложения:

```toml
[commands.review]
description = "Проверить изменения"
prompt = "Review $ARGUMENTS carefully"
```

`/review src/main.rs` готовит текст `Review src/main.rs carefully`, который клиент
отправляет обычным ходом агента. `$ARGUMENTS` — буквальная подстановка остатка
строки, не shell expansion. Команда не запускает tools сама и не выдаёт новых прав.
Имена проверяются; пустой prompt или результат expansion, дубли с host/module
командами, зарезервированные `exit`/`quit` и неизвестные поля дают ошибку.
Module-команды объявляются реализацией tool через `tool/v5`,
а не этим блоком; отключение tool убирает его команду из каталога.

Полный список текущей сборки — `/help`; [семантика и API](runtime-and-events.md#slash-команды).

## Config Builder

App-server автоматически перечитывает профиль через обычный reader: includes,
directory overlay, instructions и fixed metadata подключённых Agent Plugins.
Semantic polling выполняется каждые 300 ms; для внешней правки нужны два одинаковых
чтения и успешная подготовка сборки. Открытые сессии одного профиля подхватывают
и ручные правки файла, и сохранение из другой сессии. Изменения относятся ко всему
профилю, включая component launch/config, но не заменяют код worker-а на месте.
Текущий ход сохраняет snapshot; следующий использует новый epoch. При ошибке
остаётся последняя проверенная сборка, `ProfileReloadStatus` сообщает ошибку или
восстановление; сообщение также доступно в `/addons.reload_error` и warnings
Config Builder. Комментарий или переформатирование без изменения значения не
пересобирают runtime.

Временные model/reasoning/mode overrides сессии сохраняются при правке других
частей профиля; изменение соответствующего default применяется ко всем открытым
сессиям. Приложение обновляет селекторы и shared settings draft по server events,
сохраняя несохранённые изменения в отредактированных областях. Новые страницы
skills/MCP/packages ещё не добавлены; доступен общий сервис `agent.addons`.

Блок «Агент» в настройках приложения меняет selection, provider, permission
mode, enabled tools, hooks и `module_config`, затем сначала строит и проверяет
новый `AssemblyPlan`. Только после
успешной сборки соответствующего `PreparedAssembly` config сохраняется, а
runtime snapshot меняется одним обновлением. Он не создаёт components/exports
из воздуха: selection доступен только для entries текущего catalog. Existing
`components` и opaque `module_config` сохраняются.

`GET /config/builder` возвращает для каждого модуля nullable `config_schema`.
Модельные exports находятся в `model_modules`, обработчики — в
`hook_modules`, остальные выбираемые реализации — в `slots[].modules`.
`plugins[]` описывает настроенные process components: `id`, `command`,
nullable `description`, все `exports[]` (`slot`, `id`, `active`, описание и
`config_schema`) и `tool_packs[]` (`id` export и имена его tools).
Описание параметров `tool` и `context_provider` доступно в этих exports.
Источник — валидированный manifest его process export, а не таблица
reference module ids в Core или клиенте. Чтение описаний инициализирует
настроенные components, включая невыбранные, через их обычные shared launchers;
selection и runtime snapshot при этом не меняются. Ошибка чтения компонента
попадает в `warnings`; настройки остальных компонентов доступны. Описание
содержит порядок полей, подписи, типы, defaults, ограничения, единицы и
признак дополнительных параметров. Оно служит формам интерфейса; проверку
конфигурации при сборке по-прежнему выполняет implementation.
Просмотр defaults не добавляет их в `module_config`. Сброс поля удаляет
переопределение, а неизвестные описанию поля остаются в общем черновике.

Страница «Агент → Плагины» показывает процессные модули и их паки. Tools содержит
typed `owner: null | {component_id, module_id}` и ссылку на свой плагин/пак.
Принадлежность назначает host по configured export; `source` — только подпись,
не ключ для восстановления владельца. Выключенные process tools входят в
read-only inventory builder с `enabled = false`, `registered = false`, но
не регистрируются для исполнения и не предлагаются модели. Состояния exports,
inventory и схемы параметров читаются из одного runtime snapshot.
Переключатель пака редактирует обычный `tools_enabled` в общем черновике;
частичное включение отображается отдельно. `runtime_managed` tools этим
переключателем не управляются. Плагины не включают все exports разом:
`select_one` выбирается на странице своего slot, hooks сохраняют явный порядок.
Пакеты Agent Plugins 1.0 (`addons.plugins`, skills/MCP) — другой способ загрузки;
они показаны отдельным разделом и не превращаются в process components.

Save сериализует read/prepare/persist/publish по каноническому пути профиля;
повторное сохранение читает актуальный source, а файл заменяется atomic rename.
Все fallible проверки runtime проходят до persistence. Выбор модели и reload
используют один lock, поэтому изменение selection не может сорвать publication
уже записанного config. Принятый save завершается и при отключении HTTP caller.
Существующие ходы продолжают работу со своим immutable snapshot.

Перед записью профиля сохранение кладёт заменяемое состояние этих полей в
`<config store>/config-history/<путь профиля>/`, рядом с sessions, а не в
каталог `configs`: он может быть ссылкой на репозиторий. Хранятся последние
50 состояний; сохранение без изменений и повтор последнего состояния новую
запись не создают. Запись истории выполняется до изменения файла, и её ошибка
отменяет сохранение. `GET /config/history` возвращает эти состояния, а
страница «Агент → История изменений» загружает выбранное в черновик. Откат
проходит обычным `POST /config/builder`, поэтому проверяется той же сборкой и
сам попадает в историю. Builder не умеет снимать выбор slot: если в старом
состоянии slot не был выбран, текущий модуль остаётся. Параметры модулей,
которых не было в старом состоянии, сохраняются пустым объектом.
Ручные правки других частей файла откат не трогает.

До запуска тот же результат можно проверить отдельно:

```bash
proteus --config codex inspect plan
```

Неизвестный selection или другая блокирующая plan-проверка не запускает
модуля и не заменяет текущий runtime. Поля и ограничения описаны в
[assembly-plan.md](../architecture/assembly-plan.md).

## Проверка

```bash
PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml doctor

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml modules list

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml tools list
```

`inspect plan` не запускает components. `doctor` не отправляет model request и
не выполняет behavioral turn, но при сборке фактического tool registry может
поднять model/tool components и выполнить bootstrap `describe`/`list` и handshake.
Остальные selections он проверяет декларативно; полный strict handshake всех
активных exports проверяют conformance gate и реальная сборка runtime snapshot.

Сессии `doctor` проверяет только для выбранного `--cwd` (или текущего каталога).
Для полного аудита используйте `proteus --config codex doctor --all-sessions`.
Scope указывается в выводе; проверка остаётся строгой и не обновляет старые
session formats автоматически.
