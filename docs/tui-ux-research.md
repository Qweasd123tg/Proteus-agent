# TUI UX Research

Этот документ фиксирует разбор локальных исходников Codex, Claude Code и
OpenCode из `examples/`, а также текущих рисков `clients/tui`. Цель - понять,
почему чужие агенты ощущаются стабильнее, и какие изменения переносить в наш
TUI без протаскивания visual logic в core.

## Короткий Вывод

Основной эталон для технической архитектуры TUI - Codex. Он ближе всего к
нашему Rust/ratatui стеку и уже решает те же проблемы: retained frame diff,
active streaming cell, bottom pane, paste-burst и approval views. Claude Code и
OpenCode оставляем как UX-ориентиры для `/context`, permission flows, slash
ranking, themes и dialog details, но они не должны размывать базовую TUI
архитектуру.

Стратегическая оговорка: этот документ не означает, что TUI должен стать
главным продуктом прямо сейчас. Если проект выбирает `Kernel/Harness First`,
то TUI надо довести до честного usable reference client и остановить глубокую
полировку до появления evals/golden coding profile. Если проект выбирает
`Codex-Like TUI Product`, тогда текущий hybrid renderer стоит заменить на
retained viewport/bottom-pane архитектуру, а не продолжать бесконечно чинить
симптомы.

Проблема не в отдельных цветах или отступах. У зрелых TUI есть отдельные
state machines для terminal frame, composer, streaming, paste, dialogs,
footer/status и resize. У нас сейчас гибрид:

- завершённый transcript пишется напрямую в normal terminal scrollback;
- нижняя inline panel переписывается относительными `MoveUp`/`MoveDown`;
- fullscreen overlays используют отдельный alternate screen;
- часть состояния scroll/streaming/footer существует в `AppState`, но не всегда
  соответствует реальному terminal viewport.

Такой гибрид быстро даёт визуальные баги: resize/zoom ломает позиции, streaming
мерцает, остаточные символы остаются на экране, markdown пересчитывается
рывками, а scroll hints могут не соответствовать фактическому поведению.

## Что Делает Codex

Локальные исходники: `examples/chatgpt/codex-study/openai-codex .../codex-rs/tui/src`.

- `custom_terminal.rs` держит front/back buffers и diff-ит полный frame перед
  flush. `Terminal::try_draw` делает autoresize, рендерит полный frame, пишет
  только diff и затем выставляет cursor. Это резко снижает stale cells после
  resize и animation ticks.
- `tui.rs` не рисует нижнюю панель относительными `MoveUp` от текущего
  положения cursor. Вместо этого он держит `viewport_area`, расширяет/сжимает
  его через `update_inline_viewport`, очищает viewport через
  `Terminal::clear()` и затем делает обычный full-frame draw.
- `insert_history.rs` вставляет новые transcript lines над viewport через
  scroll-region (`SetScrollRegion`/`ResetScrollRegion`) и обновляет
  `terminal.viewport_area`. То есть история и bottom pane не борются за одну и
  ту же terminal cursor position.
- `custom_terminal.rs::diff_buffers` отдельно испускает `ClearToEnd` для хвоста
  строки и учитывает wide/multi-width glyph invalidation. Это важнее, чем
  точечные `Clear(CurrentLine)`: renderer знает, какие cells действительно
  изменились, и чистит только валидную область frame.
- `chatwidget.rs` разделяет committed transcript cells и live `active_cell`.
  Streaming мутирует active cell in place, а не перепечатывает случайные куски
  transcript.
- Streaming controller отдаёт строки дозировано: completed lines попадают в
  очередь и коммитятся постепенно. Это держит форму блока стабильной во время
  длинных ответов.
- `bottom_pane` владеет `ChatComposer` и stack-ом transient views. Input routing
  слоистый: popup/modal получает клавишу первым, затем composer, затем внешний
  ChatWidget решает cancel/quit.
- `paste_burst.rs` есть как отдельная pure state machine. Она нужна не только
  для Windows: она предотвращает ситуацию, где paste приходит как много
  `Char`/`Enter`, а `Enter` внезапно отправляет 20 сообщений.
- Footer/status в `bottom_pane/footer.rs` выбирается по ширине. Там есть
  collapse policy: сначала убрать второстепенный контекст, затем shortcut hint,
  затем перейти в короткую строку.
- Approval живёт как отдельный bottom-pane view, а не как случайная строка
  поверх composer.

Практический урок: если хотим ощущения Codex, нужен retained render path и
bottom-pane state machine. Одними `Clear(CurrentLine)` это не догнать.

## Что Делает OpenCode

Локальные исходники: `examples/opencode/source/opencode/packages/opencode/src/cli/cmd/tui`.

- `app.tsx` запускает OpenTUI renderer с `targetFps: 60`, mouse support,
  `exitOnCtrlC: false`, kitty keyboard и `externalOutputMode: "passthrough"`.
  То есть UI живёт в управляемом renderer tree.
- `component/prompt/index.tsx` делает composer полноценным компонентом:
  textarea, extmarks, file/agent/paste markers, prompt history, stash,
  command registration, image paste, editor handoff.
- `component/prompt/autocomplete.tsx` держит autocomplete как anchored popup:
  позиция пересчитывается при изменении anchor/terminal size, есть fuzzy search,
  keyboard/mouse input mode и frecency.
- `ui/dialog.tsx` даёт общий dialog stack с focus restore, selection-aware
  dismiss и Ctrl+C/Esc close.
- `ui/dialog-select.tsx` - reusable filterable picker с группировками,
  page/home/end navigation, scrollbox и высотой от terminal dimensions.
- `routes/session/permission.tsx` показывает approval как permission prompt с
  permission-specific body. Для edit есть diff preview и width-aware split vs
  unified mode.
- Theme layer семантический: UI использует tokens, а не точечные цвета.

Практический урок: slash/resume/context/approval должны быть не набором
особых случаев, а поверх общего dialog/picker слоя.

## Что Делает Claude Code

Локальные исходники: `examples/claude/claude-code-src/src`.

- `ink/renderer.ts` держит frame-engine с buffers, scroll drain и защитой от
  загрязнения терминала.
- `FullscreenLayout.tsx`, `modalContext.tsx` и `promptOverlayContext.tsx`
  отделяют bottom slot от floating overlays. Overlay не ломает composer layout.
- `PromptInput.tsx` - большой composer state machine: cursor, modes, history,
  speculative acceptance, paste collapse, overlay gating.
- `commandSuggestions.ts` держит Fuse index для slash commands, ranking по
  exact/prefix/alias/usage и cached discovery.
- `/context` строит аналитический отчёт: сначала приводит transcript к API-view,
  затем считает usage и рендерит цветной ANSI report.
- Permission UI разделён на prompt и rule management: allow/ask/deny/retry
  живут как понятные состояния.

Практический урок: Claude сильнее в debug/visibility. `/context`, permissions и
command suggestions надо делать как отдельные UX-сценарии, а не как markdown
текст, случайно показанный в transcript.

## Наши Текущие Риски

Код: `clients/tui/src`.

1. `scroll_offset` почти не влияет на normal transcript render. Клавиши
   `PageUp`/`PageDown` меняют state, но transcript уже напечатан в terminal
   scrollback через `flush_scrollback_messages`. Поэтому UI может писать, что
   есть scroll mode, хотя реальный viewport не управляется приложением.

2. `draw_inline_panel` хранит только `height` и `cursor_row`. Он не владеет
   terminal viewport и не может гарантировать, что текущая cursor position
   действительно находится там, где была после прошлого draw. После streaming
   tick, terminal scroll, wrapped line change, font zoom или mode switch
   относительный `MoveUp(...)` легко попадает не в тот physical row.

3. Мы смешали три rendering model:
   normal scrollback append, manual inline diff и alternate-screen overlay. Это
   главная причина визуальной нестабильности.

4. Последовательные short-term fixes (`Hide` cursor during redraw,
   `Clear(CurrentLine)` перед каждой строкой, расширенное очищение старой
   области panel) уменьшили часть симптомов, но не убрали артефакты полностью.
   Это подтверждает, что проблема не локальная в статусной строке `preparing`,
   а в отсутствии retained viewport contract.

5. Streaming preview каждый frame заново прогоняет растущий текст через markdown
   renderer и берёт tail. Если markdown/table/code block меняет wrap выше tail,
   пользователь видит мерцание и скачки.

6. Paste зависит от bracketed paste. Если terminal присылает быстрый поток
   `Char`/`Enter`, `Enter` может отправить несколько сообщений.

7. Footer/status строится из строк без широтной policy. В результате status,
   hints, timer и command hints начинают конкурировать за одну строку.

8. Context overlay и resume picker не имеют общего picker/dialog contract.
   Поэтому каждый новый overlay будет снова изобретать scroll, query, sizing,
   close behavior и responsive layout.

9. Markdown tables пока split-ят pipe rows слишком просто. Escaped pipes,
   inline code с `|`, очень узкие терминалы и широкие русские строки будут
   оставаться источником edge cases.

## Рекомендуемая Архитектура TUI

Core трогать не нужно. Это должен быть client/control-plane слой поверх
app-server protocol.

### 1. Выбрать Primary Render Model

Есть два варианта:

- Short-term: оставить normal scrollback и принять, что visual bugs будут
  появляться на отдельных terminal/font/zoom сценариях. Этот путь годится только
  как временный dogfood mode.
- Correct path: перейти к Codex-like retained inline viewport:
  `CustomTerminal`, `viewport_area`, `insert_history_lines`, full-frame diff
  bottom pane. Transcript lines вставляются над viewport, active bottom pane
  рисуется внутри viewport, cursor выставляется через frame contract.

Если цель - ощущение Codex, дальнейшие точечные патчи `MoveUp/Clear/Print`
нужно остановить. Они могут немного менять симптом, но архитектурно не закрывают
resize/zoom/stale-cell проблему.

### 2. Ввести BottomPane Model

Вынести из `main.rs`/`visual.rs` явную модель:

```text
BottomPane
  composer
  view_stack
  status_line
  footer
  paste_burst
  command_popup
```

`main.rs` должен только маршрутизировать app-server events и terminal events.
Composer, slash popup, approval, resume picker и context report не должны быть
разрозненными `if state.has_*`.

### 3. Разделить Transcript И Active Cell

Нужны две зоны:

- committed transcript: финальные user/assistant/tool/system cells;
- active cell: текущий streaming assistant/tool/status, который мутирует in
  place и не попадает в scrollback до commit.

Это уменьшит дублирование TurnOutput, рывки streaming tail и хаос при tool calls.

### 4. Сделать Generic Dialog/Picker

Один общий слой должен покрывать:

- `/resume`;
- `/context`;
- slash command menu;
- approval;
- будущие `/model`, `/doctor`, `/sessions`.

Минимальный contract:

```text
DialogView
  title
  query/input optional
  selected row
  scroll offset
  height policy
  key handling
  render rows
```

### 5. Composer Как State Machine

Composer должен владеть:

- text buffer и cursor;
- soft wrap по terminal width;
- paste ranges и `[Pasted Content N chars]`;
- paste-burst fallback;
- slash completion;
- history позже;
- disabled/busy mode.

Тогда Ctrl+C, Enter, Tab и paste можно сделать предсказуемыми.

### 6. Footer Collapse Policy

Footer не должен просто склеивать строки. Нужны приоритеты:

1. blocking action hint: approval/cancel/quit confirmation;
2. active command hint: slash/resume/context navigation;
3. task status: spinner, elapsed, tokens;
4. passive context: model/session/context percent;
5. generic shortcuts.

На узкой ширине низкоприоритетное содержимое пропадает первым.

### 7. Markdown Strategy

Для финальных assistant messages можно оставить markdown renderer. Для live
streaming лучше один из вариантов:

- plain incremental render до финала, затем markdown;
- line-gated markdown only for completed lines;
- incremental markdown cache по стабильным блокам.

Первый вариант проще и, вероятно, лучше для UX на ближайший этап.

## Очередь Работ

### Phase 0 - Stop The Bleeding

Первый заход уже должен идти строго в Codex-направлении: стабилизировать
terminal surface и active streaming view, не меняя core protocol.

- Fake transcript scroll hints уже убраны: `PageUp`/`PageDown` больше не
  обещают app-managed transcript scroll в normal screen.
- Заменить relative per-line diff на безопасный inline full-redraw block.
  Первый short-term вариант сделан: нижняя inline panel теперь каждый redraw
  очищает блок от начала предыдущей панели до низа и рисует актуальные строки
  заново, без сравнения отдельных строк.
- После dogfood стало видно, что full-redraw block всё ещё недостаточен:
  артефакты исчезают при terminal zoom/full repaint, но возвращаются во время
  normal redraw. Первый Codex-like шаг сделан: нижняя панель закреплена внизу
  viewport и рисуется по абсолютным координатам. После проверки на реальном
  терминале прямой `Backend::scroll_region_up` заменён на более близкую к
  Codex вставку history через `SetScrollRegion` + `\r\n` у нижней границы
  history-region, чтобы не ломать обычный terminal scrollback.
- Dogfood большого streaming output выявил ещё одну проблему промежуточного
  слоя: когда live-preview увеличивал нижнюю панель вверх, новые строки панели
  просто очищали уже нарисованный transcript. Исправлено в Codex-направлении:
  рост bottom viewport сначала прокручивает history-region на delta высоты, а
  live-preview ограничен частью экрана до полноценного retained active cell.
- Следующий промежуточный шаг сделан: inline renderer вынесен из `main.rs` в
  `clients/tui/src/inline_terminal.rs`, чтобы event loop больше не владел
  деталями viewport/history insertion. На время streaming live-preview
  резервирует стабильную высоту, поэтому длинный ответ не должен каждый frame
  поднимать bottom pane и повторно двигать transcript. Это всё ещё не полный
  `custom_terminal` Codex с front/back buffer, но теперь есть отдельная точка
  для его переноса.
- Dogfood показал ошибку порядка в промежуточном shrink-path: после окончания
  streaming финальный transcript мог вставляться в расширенную history-zone, а
  затем стираться очисткой старой высокой bottom panel. Исправлено: при
  уменьшении панели старая область очищается до вставки новых history lines, а
  последующий draw чистит только актуальную нижнюю панель.
- Следующий dogfood показал, что во время длинного streaming ответа
  промежуточный renderer всё ещё оставлял большую пустую history-область и
  показывал только короткий tail внизу. Для текущего слоя live-preview расширен
  почти на весь доступный экран, а shrink теперь делает полный repaint normal
  screen из `AppState`, чтобы пустые строки, созданные ростом bottom pane, не
  оставались после commit финального transcript.
- После проверки UX стало ясно, что сама идея держать live answer в нижней
  panel неверна: текст растёт снизу вверх и ломает ожидания. Live assistant
  убран из bottom panel. Во время streaming normal screen теперь полностью
  перерисовывает transcript/active answer сверху вниз, а bottom panel оставляет
  только status, composer и footer. Это временный repaint-path до полноценного
  Codex-style active cell, но он возвращает правильное направление роста текста.
- Для streaming repaint добавлен минимальный app-managed scroll: `PageUp`/
  `PageDown` и `Up`/`Down` двигают окно transcript, пока агент отвечает.
  Чтобы уменьшить flicker, streaming path больше не делает `Clear(All)` и
  `Purge` каждый frame, а очищает только строки history viewport перед
  перерисовкой.
- Scroll во время streaming должен быть anchored, а не offset-from-tail:
  когда пользователь ушёл вверх, новые delta-строки не двигают окно обратно к
  live tail. Renderer держит anchored visible end, а `AppState` компенсирует
  рост rendered transcript rows, пока scroll offset активен.
- Отдельный source clipping во время streaming был в markdown code block:
  строки заворачивались на всю ширину контента, а потом получали внутренний
  префикс `│ `. Теперь code block учитывает этот префикс при wrap. Следующий
  правильный шаг по Codex - line-batched streaming controller, где UI
  публикует завершённые строки, а не каждый provider delta по слогам.
- Начат разворот от `inline_terminal.rs` как mini-compositor к отдельному
  terminal layer: raw history insertion, scroll regions, terminal line writer
  и panel draw/clear вынесены в `history_insert.rs` и `terminal_surface.rs`.
  Поведение пока намеренно сохранено, цель шага - запретить дальнейшее
  размазывание terminal tricks по UI-layer.
- Второй шаг migration: введён `TranscriptStore` с разделением committed
  transcript и active streaming cell. Active assistant больше не является
  уже emitted history и не вставляется в scrollback до final/tool/cancel.
  Старый streaming full-history repaint-path удалён: во время ответа
  перерисовывается bottom/status pane, а стабильная история остаётся
  append-only.
- Третий шаг migration: active assistant снова виден во время streaming, но
  теперь как transient live-tail над bottom pane. Это не committed scrollback:
  tail рисуется внутри `TerminalSurface`, учитывается в reserved bottom height,
  поддерживает scroll offset через `PageUp`/`PageDown`/`Up`/`Down` и исчезает
  при finalization, после чего final assistant вставляется как обычный
  committed history batch.
- Четвёртый шаг migration: overlay enter/leave отделён от полного reset inline
  terminal state. При входе в alt-screen очищается только transient bottom/live
  area; при выходе сохраняется history viewport cursor, поэтому committed
  события, пришедшие пока overlay открыт, flush'ятся после возврата без
  повторного старта normal scrollback с верхней строки.
- Пятый шаг migration: resize теперь ставит pending source-backed reflow вместо
  немедленного reset normal screen из event handler. Первый следующий normal
  draw очищает owned normal screen, rewind'ит emitted cursor и replay'ит
  committed transcript под актуальную ширину. Если resize пришёл в alt-screen,
  reflow откладывается до выхода из overlay.
- Шестой шаг migration: tool history переведён в append-only режим. `Running`
  tool card теперь может быть emitted в normal scrollback сразу и не блокирует
  последующие committed messages; `ToolFinished` добавляет отдельную final
  card с `Ok`/`Err` и preview вместо mutation уже напечатанного scrollback.
- Седьмой шаг migration: добавлена первая граница `bottom_pane.rs`.
  `inline_terminal` больше не вызывает line-builder из `visual.rs` напрямую, а
  работает через `BottomPane::lines(...)`. Поведение пока сохранено; следующий
  шаг - перенос composer/status/slash/approval/footer internals из `visual.rs`
  внутрь bottom-pane module.
- Восьмой шаг migration: сборка inline bottom-pane lines переехала в
  `bottom_pane.rs`. `visual.rs` пока оставляет общие renderer/helper функции,
  но больше не владеет итоговой композицией composer/status/slash/approval/footer
  для normal inline path. Следующий шаг - перенести сами helper'ы и tests в
  bottom-pane submodules.
- Девятый шаг migration: `bottom_pane` превращён в модуль-папку с отдельными
  `composer.rs` и `footer.rs`. Сейчас это ещё тонкие boundary-модули, но
  раскладка уже отделена от `visual.rs`; следующий шаг - вынести remaining
  visibility/layout helpers для slash/approval/status из общего файла.
- Десятый шаг migration: `live_preview` вынесен в отдельный модуль. `visual.rs`
  больше не владеет streaming/live-tail layout целиком и использует внешний
  helper для расчёта высоты и рендера preview. Следующий шаг - вынести
  approval/header helpers в отдельные files и дальше сокращать `visual.rs`.
- Одиннадцатый шаг migration: карточки `session_card` и approval lines вынесены
  в `cards.rs`. `inline_terminal` теперь берёт scrollback header напрямую из
  нового boundary, а `visual.rs` держит только оставшиеся визуальные helpers.
  Следующий шаг - отделить footer/status/detail helpers, чтобы продолжать
  сжимать `visual.rs`.
- Двенадцатый шаг migration: статусная строка и reasoning visibility переехали
  в `bottom_pane/status.rs`. `bottom_pane` теперь не тянет active-status logic
  из `visual.rs`, а использует отдельный helper module. Следующий шаг - добить
  оставшиеся layout helpers (`composer`/gaps) и убрать дублирующие определения
  из `visual.rs`.
- Тринадцатый шаг migration: live tail в `inline_terminal` временно переведён
  на top-anchored render, чтобы первые строки streaming-ответа не исчезали при
  росте вывода. Это убирает ощущение, что ответы "едят" начало текста; дальше
  можно отдельно вернуть follow-tail режим как опцию, если он понадобится.
- Четырнадцатый шаг migration: top-anchored live tail оказался тупиком для
  длинных ответов. Active assistant больше не рисуется отдельным bottom
  live-preview: завершённые строки streaming-ответа вставляются в normal
  history по мере появления, а финальный `TurnOutput` пропускает уже
  вставленные строки и дописывает только остаток. Это переводит длинный вывод
  с нижнего preview-path на retained/history-path.
- Пятнадцатый шаг migration: line-batched streaming не должен вставлять в
  scrollback открытые markdown table blocks. Таблица рендерится только когда
  block закрыт пустой строкой/следующим нетабличным block или финальным
  `TurnOutput`; иначе нижняя рамка уже вставленной таблицы остаётся в history
  и следующие rows выглядят как "обрезанные".
- Шестнадцатый шаг migration: markdown renderer получил поддержку
  `==highlight==`, `__bold__`, `_italic_`, многоуровневых blockquote markers
  (`>`, `>>`, `>>>`) и лёгкую syntax coloring для fenced code blocks. Это не
  полноценный GitHub Markdown/parser, но закрывает основные dogfood-разрывы в
  цитатах и кодовых блоках.
- Семнадцатый шаг migration: highlight убран с background fill и стал
  foreground-only для тёмных terminal themes; raw ``` fences заменены на
  компактный `code · lang` label без closing fence; добавлен terminal render
  для autolinks `<https://...>`, bare URLs и footnote refs.
- Восемнадцатый шаг migration: normal markdown links теперь показывают только
  label без URL шума, autolinks/bare URLs продолжают показывать URL, а images
  рендерятся как компактный alt text без длинного source URL.
- Девятнадцатый шаг migration: inline markdown теперь парсится до wrap, а не
  после него. Это предотвращает raw backticks у code spans вроде
  `remember_fact`, если token попадает на границу переноса строки.
- Двадцатый шаг migration: headings теперь тоже используют inline markdown
  parsing. Заголовки вида `## 🧠 \`remember_fact\`` больше не показывают raw
  backticks в chat transcript.
- Двадцать первый шаг migration: tool cards получили status-colored markers и
  less-muted transcript styling. `Running` отображается жёлтым, `Ok` зелёным,
  `Err` красным; грубый `✗ Failed` заменён на красный `● Error`, а action/output
  styles вынесены в общие helpers для scrollback и live-preview.
- Исправить context overlay scroll direction.
- Ограничить streaming markdown: live plain text, final markdown или
  block-aware staging для table/code/quote blocks.
- Добавить snapshot tests для размеров 60x20, 80x24, 120x30.

### TUI Migration Checklist

- `TerminalSurface` владеет raw terminal operations. UI-компоненты не должны
  напрямую использовать `MoveTo`, `Clear`, `Print`, `SetScrollRegion`.
- `TranscriptStore` владеет source of truth: committed cells отдельно,
  active cell отдельно, emitted cursor отдельно.
- Active streaming cell не пишет partial chunks в terminal scrollback.
  Разрешены только line-batched stable rows: завершённые строки active
  assistant вставляются в history, финал дописывает остаток без дублей.
- Bottom pane должен стать Ratatui component stack: composer, status, slash,
  approval, footer без знания о scroll regions.
- Overlay/alt-screen должен defer'ить history insertion и flush'ить её после
  возврата в normal screen.
- Resize reflow должен идти из `TranscriptStore`, а не из terminal contents.

### Phase 1 - BottomPane

- Вынести `BottomPaneModel` и `ComposerModel`.
- Перенести slash popup, approval и footer policy в bottom-pane слой.
- Сделать footer collapse rules.
- Добавить paste-burst fallback по мотивам Codex.

### Phase 2 - Dialog/Picker

- Общий picker для `/resume`, slash commands и будущих menus.
- Общий fullscreen/overlay renderer для `/context`.
- Approval как view поверх того же dialog/picker contract.

### Phase 3 - Retained Transcript

- Ввести committed cells + active cell.
- Перестать печатать active streaming text в terminal scrollback.
- Добавить управляемый transcript viewport или Codex-like inline viewport с
  buffer diff.

### Phase 4 - Polish

- Semantic theme tokens.
- Diff preview в approval.
- Context report как Claude-like colored map с category legend.
- Prompt history, command aliases, fuzzy suggestions.

## Что Не Делать

- Не переносить TUI state в core.
- Не добавлять новый core slot ради slash menu, markdown, resume picker или
  approval UI.
- Не копировать Codex/OpenCode/Claude целиком. Переносить надо паттерны:
  retained render, bottom pane, composer state machine, dialog stack,
  paste-burst и footer collapse.

## Критерий Готовности

TUI можно считать вышедшим из demo-grade, когда проходят сценарии:

- resize/zoom во время streaming не оставляет мусор и не ломает cursor;
- длинный streaming answer виден стабильно до финала;
- paste большого многострочного текста становится одним input с placeholder;
- `/resume`, `/context`, slash menu и approval используют один понятный
  overlay/dialog паттерн;
- footer не ломается на узком терминале;
- markdown tables/code blocks не выходят за width;
- `cargo test -p agent-tui` покрывает visual state и layout edge cases.
