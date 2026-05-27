use std::io;

use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute, queue,
    terminal::{
        BeginSynchronizedUpdate, Clear as TerminalClear, ClearType, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    inline_terminal::InlineTerminalState, state::AppState, terminal_surface::TerminalSurface,
    visual::VisualSurface,
};

pub(crate) type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub(crate) fn install_panic_hook() {
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste,
            crossterm::event::DisableMouseCapture,
        );

        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = format!("=== TUI panic ===\n{info}\n\nbacktrace:\n{backtrace}\n",);

        eprintln!("{msg}");

        let path = std::env::temp_dir().join("proteus-tui-panic.log");
        let _ = std::fs::write(&path, &msg);
        eprintln!("panic log: {}", path.display());
    }));
}

pub(crate) fn enter_terminal() -> Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    // Основной чат живёт в normal screen: завершённые сообщения пишутся в
    // настоящий terminal scrollback, поэтому выделение мышью и wheel работают
    // так же, как в shell. Mouse capture и alternate scroll здесь не включаем.
    execute!(
        out,
        EnableBracketedPaste,
        MoveTo(0, 0),
        TerminalClear(ClearType::All),
        TerminalClear(ClearType::Purge)
    )?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

pub(crate) fn leave_terminal(terminal: &mut TuiTerminal) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        TerminalClear(ClearType::FromCursorDown)
    )?;
    terminal.show_cursor()?;
    Ok(())
}

pub(crate) fn reset_normal_screen(
    terminal: &mut TuiTerminal,
    state: &mut AppState,
    header_printed: &mut bool,
    inline_terminal: &mut InlineTerminalState,
) -> Result<()> {
    TerminalSurface::new(terminal).clear_normal_screen(true)?;
    state.rewind_scrollback();
    *header_printed = false;
    inline_terminal.reset();
    Ok(())
}

pub(crate) fn redraw(
    terminal: &mut TuiTerminal,
    surface: &VisualSurface,
    state: &mut AppState,
    scrollback_header_printed: &mut bool,
    inline_terminal: &mut InlineTerminalState,
    picker_alt_screen: &mut bool,
) -> Result<()> {
    queue!(terminal.backend_mut(), Hide, BeginSynchronizedUpdate)?;
    let result = (|| -> Result<()> {
        if state.has_fullscreen_overlay() {
            if !*picker_alt_screen {
                inline_terminal.enter_overlay(terminal)?;
                execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                terminal.clear()?;
                *picker_alt_screen = true;
            }
            terminal.draw(|frame| surface.render_overlay(frame, &state.visual_state()))?;
        } else {
            if *picker_alt_screen {
                execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                *picker_alt_screen = false;
                inline_terminal.leave_overlay();
            }
            inline_terminal.draw_normal(terminal, state, scrollback_header_printed)?;
        }
        Ok(())
    })();
    let finish_result = queue!(terminal.backend_mut(), EndSynchronizedUpdate, Show)
        .and_then(|_| std::io::Write::flush(terminal.backend_mut()));
    result?;
    finish_result?;
    Ok(())
}
