# Reference Modules

Здесь лежат implementations, которыми Proteus проверяет contracts и собирает
текущие dogfood profiles. Это не стандартная библиотека модулей, не набор
обязательных defaults и не привилегированный слой runtime.

Все реализации экспортируются через один исполняемый
`proteus-reference-worker`. Наличие crate в этой папке само по себе не означает
активацию: config явно описывает process command и выбирает конкретный
`module_id`.

Reference worker использует тот же публичный process protocol, что и внешний
worker, проходит тот же conformance gate и не получает исключений по имени или
расположению исходников. Crates в этом каталоге — только implementation detail
этого executable, а не особый способ загрузки модулей.
