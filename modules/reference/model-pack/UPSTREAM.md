# Происхождение OpenAI Codex Subscription Adapter

`openai_codex` — provider implementation для ChatGPT OAuth и подписочного
Responses endpoint. Это адаптация сетевого пути, а не копия всего runtime
Codex или OpenCode.

## Зафиксированные Источники

- OpenCode `77429f59823c8c6df9cfee95d4c663043b017f46`:
  [plugin/openai/codex.ts](https://github.com/anomalyco/opencode/blob/77429f59823c8c6df9cfee95d4c663043b017f46/packages/opencode/src/plugin/openai/codex.ts).
  Browser OAuth с PKCE/state, device-code endpoints, client id, token exchange,
  account id, Bearer/ChatGPT-Account-Id и `/backend-api/codex/responses`.
- Codex `0bbea86a6aae37b1f243676db4248000f04ad111`:
  [login](https://github.com/openai/codex/tree/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/login/src),
  [Models endpoint](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/codex-api/src/endpoint/models.rs),
  [Responses request](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/codex-api/src/common.rs),
  [quota endpoint](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/backend-client/src/client/rate_limit_resets.rs),
  [quota mapping](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/backend-client/src/client.rs).
  Проверка auth/backend boundary и набора полей запроса. Общий Responses/SSE
  mapper Proteus сохраняет собственную ранее закреплённую provenance.
- [Официальная авторизация Codex](https://learn.chatgpt.com/docs/auth):
  ChatGPT subscription access отличается от API-key billing.
- [Модели Codex](https://learn.chatgpt.com/docs/models), проверено 2026-09-09:
  профиль выбирает `gpt-5.6-luna`; доступность зависит от аккаунта.

- [Codex app-server: rate limits](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt),
  проверено 2026-09-10: `account/rateLimits/read` предоставляет отдельное чтение
  лимитов и несколько buckets через `rateLimitsByLimitId`. Proteus обращается
  к provider HTTP напрямую, не запускает Codex и не копирует его клиентский RPC DTO.

## Граница Адаптации

Proteus владеет своим workflow, tools, canonical history и journal. Provider
владеет OAuth, credentials и HTTP. Core не получает OpenAI auth DTO или tokens.
Client id соответствует OAuth-приложению Codex, как в OpenCode; отдельная
сессия Proteus не означает регистрацию нового OAuth-приложения.

Явные особенности этой реализации:

- Отдельный файл Proteus, без чтения или импорта credentials Codex/OpenCode.
  Атомарная запись и OS file lock координируют refresh между workers/peers.
  Нет keyring backend или общего host credential service.
- Access token обновляется за 60 секунд до expiry и один раз после HTTP 401.
  Refresh не повторяется вслепую. Уже начатая bounded refresh-транзакция
  завершается при отмене model invocation, пока жив worker; это предотвращает
  потерю нового refresh token. Аварийное завершение самого процесса между
  ротацией на сервере и записью файла всё ещё может потребовать нового входа.
- Login ограничен 15 минутами, отдельный HTTP auth request — 30 секундами;
  device polling ждёт server interval плюс 3 секунды. HTTP redirects отключены.
- `originator`/User-Agent обозначают Proteus. Подписочный endpoint получает
  `store=false`, `stream=true`, без `max_output_tokens`. Как в OpenCode, этот
  output limit не отправляется провайдеру; локальные deadline/cancellation
  сохраняются. `stream=false` и внутренний complete собирают terminal из SSE
  без доставки промежуточных events; это не второй inference request.
- API credentials, non-stream error fallback и `prompt_cache_retention` в
  subscription config отклоняются. При 429 нет повторов, переключения на API
  или другую модель. Общие Responses error/canonical semantics сохраняются.
- Каталог запрашивается отдельным `GET /models?client_version=<semver Proteus>`
  с теми же OAuth headers и однократным refresh после 401. Все remote entries
  сохраняются, `visibility` становится `hidden`, порядок задаёт `priority`.
  Из catalog не импортируются base instructions, tools или capabilities.
  В отличие от полного Codex ModelsManager здесь только memory cache на
  5 минут и общий deadline 30 секунд: нет disk cache, embedded model list,
  ETag merge или stale fallback при ошибке.
- Квота читается из `/backend-api/wham/usage`. Основная группа `codex` и все
  `additional_rate_limits` сохраняют свои окна, `used_percent`, длительности в
  секундах, абсолютные reset timestamps и признаки доступности; optional credits
  передаются без денежной интерпретации. Поля будущих provider HTTP ответов,
  не представленные в общем quota DTO, не публикуются в Core.
  Собственная адаптация Proteus: memory cache 30 секунд с coalescing, deadline
  30 секунд, без stale fallback. Cache hit сохраняет `observed_at`. HTTP 401
  допускает один refresh; другие ошибки возвращаются без тела ответа и повторов.
  Отдельный `quota_url` нужен для explicit endpoint override, а не выводится из
  Responses `base_url`. Дополнительных действий со spend/reset endpoint нет.
- `logout` удаляет только локальные credentials Proteus. `status` показывает
  наличие и срок access token, не проверяет доступность модели или остаток
  подписки по сети.

Browser/device OAuth, refresh и сетевой contract проверяются loopback fixtures.
Process regression проверяет tool loop, journal, cold resume и workflow replay.
Эти проверки не доказывают доступ конкретного аккаунта к live backend; первый
настоящий вход и model request требуют аккаунта владельца.
