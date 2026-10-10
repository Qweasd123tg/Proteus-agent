# Tool Search На tgrep

`tool.py` — внешний компонент с единственным export `tool/tgrep_search`,
который предоставляет tool `search`. Отдельного search slot и context provider
здесь нет. Core, canonical DTO и обычный tool safety/approval path не меняются.
Python нужен только для этого примера; runtime contract не зависит от языка.

Нужны Python 3 и [tgrep](https://github.com/microsoft/tgrep) в `PATH`.
Пример проверен с tgrep 1.1.0. Его binary не входит в поставку Proteus.

## Подключение

В существующем профиле удалите export `tool/rg_search`, затем добавьте:

```toml
[components.tgrep-search]
command = "python3"
args = ["-B", "/absolute/path/to/examples/modules/tgrep-search/tool.py"]

[components.tgrep-search.exports.tool.tgrep_search]
timeout_ms = 65000

[tools]
enabled = ["search"] # В рабочем профиле сохраните также остальные нужные tools.
```

Два implementations имени `search` одновременно подключать нельзя.
`components` merge-ится рекурсивно: добавление tgrep поверх include не удаляет
унаследованный `tool/rg_search`. Уберите его в исходном fragment либо используйте
самостоятельный [пример профиля](../../configs/proteus.tgrep-search.example.toml).
Reference context provider `rg_search`, если он нужен, выбирается независимо
и может оставаться подключённым. Новый tool его не включает и не заменяет.

Настройки реализации необязательны:

```toml
[module_config.tool.tgrep_search]
binary = "/path/to/tgrep" # По умолчанию tgrep из PATH.
index_path = "/path/to/index" # По умолчанию .tgrep в cwd вызова.
```

Относительный путь `index_path` отсчитывается от cwd вызова. Процессный tool
не запускает сервер и не строит индекс сам. Для повторных быстрых запросов
в отдельном терминале запустите сервер на корне проекта:

```bash
tgrep serve . --max-filesize 1M
```

Если задан `index_path`, передайте серверу тот же `--index-path`.
Без сервера можно один раз выполнить `tgrep index . --max-filesize 1M`;
после изменений такой индекс нужно обновить. Добавьте `.tgrep/` в `.gitignore`
проекта, если индекс хранится там. Для индекса вне проекта используйте
`--index-path`. Запускайте Proteus с cwd того же корня: другой cwd имеет
собственный выбор индекса. Без пригодного индекса/сервера tgrep сканирует файлы.

## Вызов

```json
{"query":"ContextBuilder","max_results":20,"starts_with":["crates/"],"ends_with":[".rs"]}
```

Сохранены args текущего `search`: `query`, `max_results`, `use_case`,
`starts_with`, `ends_with`. Запрос — regex Rust engine; расширенные выражения
с lookaround/backreferences не включаются автоматически. Prefix/suffix —
строковые фильтры относительного пути; лимит применяется после них.
Размер файла ограничен 1 MiB. Пустой запрос или нулевой лимит возвращает пустой
успешный результат; ошибки regex/запуска и timeout возвращаются как ошибки.
Отмена останавливает query process и сохраняет работоспособность компонента.

Дополнительный аргумент `freshness` принадлежит схеме этого tool:

- `indexed` (по умолчанию): обычный поиск tgrep с доступным индексом/сервером;
  обновления индекса могут отставать от файлов;
- `current`: `--no-index`, чтение текущих файлов для проверки свежих правок.

`freshness` в metadata показывает выбранный режим, а не доказанную свежесть
индекса. `limit_reached` означает достижение лимита, не точный подсчёт всех
совпадений. Structured chunks находятся в `ToolResult.metadata.chunks`,
текстовый output содержит `path:line: content`.

## Проверки

```bash
python3 -B examples/modules/tgrep-search/tests.py
# Если binary вне PATH:
TGREP_BINARY=/path/to/tgrep python3 -B examples/modules/tgrep-search/tests.py
```

Проверяются strict handshake/config, discovery, args/errors, cancellation,
timeout, совпадение scan/index, фильтры до лимита, чтение свежих добавлений,
изменений и удалений и обновление результатов сервером. Без tgrep реальные
index cases помечаются skipped;
protocol/error/cancel/timeout cases работают без него.

Проверка через существующий Rust broker, без модели и полного приложения:

```bash
target/debug/proteus-component-conformance \
  --component-id tgrep-search \
  --export '{"slot":"tool","module_id":"tgrep_search","contract_version":"v5","module_config":{}}' \
  --probe-export tool/tgrep_search --probe-method list --probe-params null \
  -- python3 examples/modules/tgrep-search/tool.py
```

Этот probe проверяет protocol/discovery. Для выполнения поиска используйте
`--probe-method invoke` и canonical tool request из `tests.py`; discovery
сам по себе не доказывает работу индекса или скорость поиска.
