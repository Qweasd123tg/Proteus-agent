# Вход С Изображениями

В приложении кнопка «Прикрепить изображения» добавляет файлы к сообщению.
Изображение также можно вставить из буфера обмена (Ctrl+V в поле ввода) или
перетащить файлы в любую точку чата. Если в буфере вместе с картинкой есть
текст, как при копировании ячеек таблицы, вставляется текст. WebKitGTK не
передаёт картинки буфера в событие вставки, поэтому на Linux приложение
читает их через свою оболочку (`read_clipboard_image`). Перетаскивание
неподдерживаемого файла показывает ошибку и ничего не прикрепляет.
Перед отправкой видны миниатюры; каждый файл можно убрать. Допускается
сообщение без текста. Пока файлы читаются, отправка недоступна.
В очереди виден счётчик изображений; правка текста сохраняет вложения.

Поддерживаются PNG, JPEG, WebP и GIF, до четырёх файлов суммарным размером
до 5 МиБ на сообщение. Ограничения конкретной модели, включая обработку
анимированных GIF, остаются правилами её API. Автоматического OCR или замены
изображения текстом нет. Аудио и видео в этот вход пока не входят.

## Модель

Общий `ModelCapabilities.supports_image_input` определяет поддержку изображений
для выбранного `ModelRef`. Core отклоняет запрос с изображением к текстовой
модели до вызова provider implementation.

OpenAI Responses, OpenAI-compatible Responses и подписочный `openai_codex`
adapter кодируют изображение как `input_image` с Base64 data URL. Для них
поддержка объявляется явно в module config:

```toml
[module_config.model.openai.capabilities]
supports_image_input = true
```

Вместо `openai` нужен exact id выбранного model export. Поставляемые фрагменты
ChatGPT включают это свойство; общая основа OpenAI-compatible proxy остаётся
консервативной, пока конкретный профиль явно не объявит поддержку vision.
Anthropic Messages adapter объявляет поддержку и отправляет блок `image`
с `source.type = "base64"`. Reference `fake` остаётся текстовым.

## Вход И Хранение

HTTP `SendRequest` и stdio `send` принимают текст и необязательный массив
`images`. Элемент — `ImageAttachment { name, mime_type, data }`, где `data`
содержит Base64 байтов файла. Для текстового сообщения массив пуст.
В Rust полный вход — `UserMessageInput`; `AgentRuntime::run_input` использует
тот же admission и Turn lifecycle, что текстовый `run`.

Host проверяет формат и общий размер до открытия Turn, сохраняет байты в
`<session_dir>/images/<sha256>` и создаёт canonical `ContentPart::Image`
с `ImageRef { id, name, mime_type, path }`. Без session persistence хранилище
располагается под корнем конфигурации либо workspace. Только наличие session
store обеспечивает cold history и resume.

Workflow получает изображения в canonical history, а `AgentTask.text`
содержит текст поручения. Слот и алгоритм workflow сохраняются. Через process
boundary и журнал идут ссылки, поэтому изображения не расходуют лимит
8 МиБ внутренних frames. Байты читает выбранная model implementation при
кодировании запроса; reference implementations используют один общий helper.
Права process exports и общий tool safety path не меняются.

Ссылки сохраняются в canonical history и journal. После перезапуска приложение
получает их в `AppTranscriptMessage.images`; авторизованный `/image` возвращает
байты по `session_dir` и `path=<sha256>`. Здесь `path` является идентификатором,
а не произвольным filesystem path. Исходные файлы владельца не нужны для resume.

При сжатии истории выбранные recent user messages сохраняют свои image parts,
включая случай сокращения текста. Сжатие может исключить старое сообщение по
правилам compactor; это не превращает изображение в текст. Workflow replay
использует записанные model outcomes и не повторяет provider invocation.
Ошибка capability validation возникает до model exchange; она сохраняется
в `TurnSettled` и cold history. Для отказов до записи model exchange текущий
workflow replay не имеет oracle и не подтверждает их эквивалентность.
