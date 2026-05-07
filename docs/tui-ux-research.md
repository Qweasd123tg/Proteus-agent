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

2. `draw_inline_panel` хранит только `height`, `cursor_row` и прошлые строки.
   Он не знает абсолютный top/bottom панели. После resize, terminal scroll,
   wrapped line change или mode switch относительный `MoveUp(previous.cursor_row)`
   может попасть не туда.

3. Мы смешали три rendering model:
   normal scrollback append, manual inline diff и alternate-screen overlay. Это
   главная причина визуальной нестабильности.

4. Streaming preview каждый frame заново прогоняет растущий текст через markdown
   renderer и берёт tail. Если markdown/table/code block меняет wrap выше tail,
   пользователь видит мерцание и скачки.

5. Paste зависит от bracketed paste. Если terminal присылает быстрый поток
   `Char`/`Enter`, `Enter` может отправить несколько сообщений.

6. Footer/status строится из строк без широтной policy. В результате status,
   hints, timer и command hints начинают конкурировать за одну строку.

7. Context overlay и resume picker не имеют общего picker/dialog contract.
   Поэтому каждый новый overlay будет снова изобретать scroll, query, sizing,
   close behavior и responsive layout.

8. Markdown tables пока split-ят pipe rows слишком просто. Escaped pipes,
   inline code с `|`, очень узкие терминалы и широкие русские строки будут
   оставаться источником edge cases.

## Рекомендуемая Архитектура TUI

Core трогать не нужно. Это должен быть client/control-plane слой поверх
app-server protocol.

### 1. Выбрать Primary Render Model

Есть два варианта:

- Short-term: оставить normal scrollback, но сделать нижнюю panel честным
  inline full-redraw блоком: не diff-ить отдельные строки, а каждый frame
  возвращаться к началу предыдущей панели, чистить её хвост и рисовать заново.
  Абсолютный bottom-anchor в normal screen нельзя использовать без retained
  viewport: он конфликтует с настоящим terminal scrollback.
- Long-term: перейти к Codex-like retained viewport: transcript, active cell и
  bottom pane живут в одном full-frame/diff renderer. Тогда scroll, resize,
  streaming и overlays становятся управляемыми приложением.

Если цель - ощущение Codex, long-term вариант правильнее. Short-term годится как
стабилизация, чтобы перестать ловить stray pixels прямо сейчас.

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
- Исправить context overlay scroll direction.
- Ограничить streaming markdown: live plain text, final markdown.
- Добавить snapshot tests для размеров 60x20, 80x24, 120x30.

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
