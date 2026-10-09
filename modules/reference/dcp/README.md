# DCP: Context View И `compress`

Самостоятельный процессный модуль Node.js 22+, а не UI-расширение и не
реализация слота `compactor`. Один component предоставляет `hook/hook.dcp`
и `tool/dcp.tools`; права каждого export остаются правами его slot.

## Подключение

```bash
cd modules/reference/dcp
npm ci --ignore-scripts
npm run build
npm test
```

Пример профиля: [proteus.dcp.example.toml](../../../examples/configs/proteus.dcp.example.toml).
Замените путь к `dist/worker.js` абсолютным путём вашей сборки. При переносе
модуля сохраняйте `dist`, `package.json`, lock и `node_modules` либо выполните
`npm ci` на целевой машине: tokenizer загружает собственный WASM из зависимостей.
Автоустановки в `install.sh` и portable-приложении нет.

Добавьте `hook.dcp` в явную цепочку `modules.hooks` и `compress`, `dcp` в
`tools.enabled`, сохранив остальные нужные инструменты. Настройки
`module_config.hook."hook.dcp"` и `module_config.tool."dcp.tools"` должны быть
одинаковыми; разные параметры одного состояния отклоняются при initialize.
Схема настроек принадлежит модулю и публикуется каждым export.

## Поведение И Происхождение

Алгоритмы и prompts импортируются из **`@tarquinen/opencode-dcp@3.2.0`**,
зафиксированного lock-файлом. Его `lib/` совпадает с upstream commit
`f8232fde1e63c2251687e4d9634bd53ce11568cb`. Источник:
[opencode-dynamic-context-pruning](https://github.com/Tarquinen/opencode-dynamic-context-pruning).

Модель получает исходный upstream prompt, ссылки на сообщения и tool
`compress` с summary, которую пишет сама. Поддерживаются исходные `range`
и `message` modes, проверки диапазонов и вложенных блоков, protected content,
deduplication и purgeErrors. Автоматические стратегии запускаются в upstream
compression pipeline, а не добавляются произвольно перед каждым запросом.
Hook применяет сохранённое состояние к следующему model request. Исходные
canonical messages остаются в journal/history: меняется только outgoing view.
Обычный compactor выбранного профиля остаётся отдельным механизмом и может
создать настоящий history checkpoint.

### Пользовательское Управление

Enabled tool `dcp` объявляет `/dcp` в общем каталоге команд, но имеет
`model_visible: false`: модель видит только `compress`.

```text
/dcp stats
/dcp context
/dcp decompress
/dcp decompress 1
```

`stats` показывает статистику, `context` — оценку контекста, `decompress` без
аргумента — активные блоки, с номером — восстановление исходного view.
Используются оригинальные upstream handlers, включая сообщения об отсутствующем
или неактивном блоке; результат показывается пользователю, не вставляется в
разговор. Изменяется package-owned prune state, не canonical history.
Команда работает без LLM/Turn через обычные tool policy/approval/cancellation;
app-server требует idle session. Остальные upstream команды не предоставляются.

## Адаптация К Proteus

Это перенос механизма DCP, **не совместимость со всей оболочкой OpenCode**:

- `hook/v4` получает read-only snapshot текущего разговора; `tool/v5` читает
  его через `host.conversation.read` с invocation-bound source message id,
  а пользовательское управление — через `host.conversation.snapshot`
  с bound session id, без требования assistant call.
  Произвольного чтения чужих sessions и записи истории нет.
- Canonical ToolCall/ToolResult проецируются в upstream tool parts, затем
  восстанавливаются без потери identity существующих сообщений. Изменённые
  request-only parts получают новые ids; неизменённые media/reasoning сохраняются.
- Используется aggregate canonical usage, а не несуществующие provider-specific
  счётчики OpenCode; доступный предел — shaped `max_input_tokens` текущего request.
- Вызов проходит обычные ToolRegistry, policy, approval и safety. Настройка
  `compress.permission` не принимается: разрешения принадлежат сборке Proteus.
- Полный каталог OpenCode commands, TUI notifications, management RPC, manual
  mode, prompt override files и расширение результатов subagents не предоставляются.
  Неподдержанные config keys отклоняются, реального OpenCode client нет.
- Platform adapters заменяют только logging, bundled prompt loading, notifications
  и persistence. Bundler открывает upstream defaults/merge без вызова его
  OpenCode config loader. Сами pruning/compression algorithms не переписаны.

Допустимые настройки: `state_dir`, `debug`, `compress`, `strategies`,
`turnProtection`, `protectedFilePatterns`; вложенные ключи проверяет upstream.
Defaults upstream сохранены, включая отключённые `protectTags` и
`protectUserMessages`. Например, `compress = { mode = "range", protectTags = true }`
явно включает сохранение текста в `<protect>`.

Tool-name patterns используют реальные имена tools вашей сборки, например
`compress.protectedTools = ["write_file", "edit_file"]`. `protectedFilePatterns`
сохраняет upstream правила извлечения путей (`filePath`, `read/write/edit.path`,
`patch.patchText`); произвольные названия/DTO tools автоматически не распознаются.

## Хранение И Отмена

По умолчанию состояние находится в
`$XDG_DATA_HOME/Proteus-agent/modules/dcp` (либо `~/.local/share/...`), отдельно
от OpenCode и portable-каталога. `state_dir` задаёт другой абсолютный путь.
Namespace — session id, а не меняющийся при cold resume thread id.

Операции одной session сериализованы; hook/tool делят только внутреннее
состояние component, не объединённые host-права. Состояние публикуется через
temporary file и atomic rename после успешной операции. Ошибка или отмена до
commit не публикует изменения. Повреждённый/старый формат завершается явной
ошибкой, без migration readers. Перезапущенный component читает сохранённые
blocks; workflow replay использует записанные outcomes и не вызывает DCP.
Одновременная запись одной session разными DCP components не поддерживается.

## Лицензия И Проверки

Модуль и импортируемый upstream — **AGPL-3.0-or-later**; upstream license
копируется в `dist/LICENSE.upstream`. При распространении сохраняйте лицензии
и предоставляйте соответствующие исходники согласно AGPL. Этот отдельный
executable не линкуется в Rust reference-module.

Overrides обновляют только неиспользуемое здесь upstream TUI dependency tree
(`solid-js`, `seroval`, `@babel/core`), не DCP algorithms/version.

`npm test` проверяет protected content, оба modes, fresh-state persistence,
управление blocks, ошибки, отмену и wire multiplexing. `scripts/test.py full` готовит свежую
Node-сборку до Rust tests; `hook_runtime` дополнительно проверяет actual
model requests, cold history и replay `Success`/`Error` без повторных effects.
