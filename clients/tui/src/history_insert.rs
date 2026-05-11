use std::{fmt, io};

use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn},
    queue,
    style::{Attribute, Color as CTermColor, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear as TerminalClear, ClearType},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color as RColor, Modifier, Style},
    text::Line,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Default)]
pub(crate) struct HistoryViewportState {
    occupied_rows: u16,
}

impl HistoryViewportState {
    pub(crate) fn occupied_rows(&self) -> u16 {
        self.occupied_rows
    }

    pub(crate) fn clamp_to_height(&mut self, height: u16) {
        self.occupied_rows = self.occupied_rows.min(height);
    }

    pub(crate) fn next_insert(&mut self, height: u16) -> Option<HistoryInsert> {
        if height == 0 {
            return None;
        }
        if self.occupied_rows < height {
            let row = self.occupied_rows;
            self.occupied_rows += 1;
            Some(HistoryInsert { row, scroll: false })
        } else {
            Some(HistoryInsert {
                row: height - 1,
                scroll: true,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HistoryInsert {
    pub(crate) row: u16,
    pub(crate) scroll: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ViewportGrowth {
    pub(crate) previous_top: u16,
    pub(crate) scroll_by: u16,
}

pub(crate) fn viewport_growth_scroll(
    screen_height: u16,
    previous_height: u16,
    next_height: u16,
    occupied_history_rows: u16,
) -> Option<ViewportGrowth> {
    let previous_height = previous_height.min(screen_height);
    let next_height = next_height.min(screen_height);
    if next_height <= previous_height {
        return None;
    }

    let previous_top = screen_height.saturating_sub(previous_height);
    let next_history_height = screen_height.saturating_sub(next_height);
    let needed_scroll = occupied_history_rows.saturating_sub(next_history_height);
    let scroll_by = (next_height - previous_height)
        .min(previous_top)
        .min(needed_scroll);
    (scroll_by > 0).then_some(ViewportGrowth {
        previous_top,
        scroll_by,
    })
}

pub(crate) fn insert_scrollback_line(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    line: &Line<'_>,
    width: usize,
    history_viewport: &mut HistoryViewportState,
    history_height: u16,
) -> Result<()> {
    let Some(insert) = history_viewport.next_insert(history_height) else {
        return Ok(());
    };
    if insert.scroll {
        queue!(
            terminal.backend_mut(),
            SetScrollRegion(1..history_height),
            MoveTo(0, history_height - 1),
            Print("\r\n")
        )?;
        write_terminal_line_without_newline(terminal, line, width)?;
        queue!(terminal.backend_mut(), ResetScrollRegion)?;
        return Ok(());
    }
    queue!(terminal.backend_mut(), MoveTo(0, insert.row))?;
    write_terminal_line_without_newline(terminal, line, width)?;
    Ok(())
}

pub(crate) fn scroll_history_for_panel_growth(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    previous_height: u16,
    next_height: u16,
    occupied_history_rows: u16,
) -> Result<()> {
    let size = terminal.size()?;
    let Some(growth) = viewport_growth_scroll(
        size.height,
        previous_height,
        next_height,
        occupied_history_rows,
    ) else {
        return Ok(());
    };

    queue!(
        terminal.backend_mut(),
        SetScrollRegion(1..growth.previous_top),
        MoveTo(0, growth.previous_top - 1)
    )?;
    for _ in 0..growth.scroll_by {
        queue!(terminal.backend_mut(), Print("\r\n"))?;
    }
    queue!(terminal.backend_mut(), ResetScrollRegion)?;
    Ok(())
}

pub(crate) fn clear_rows(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    start: u16,
    end: u16,
) -> Result<()> {
    for row in start..end {
        queue!(
            terminal.backend_mut(),
            MoveTo(0, row),
            TerminalClear(ClearType::CurrentLine)
        )?;
    }
    Ok(())
}

pub(crate) fn clear_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    purge: bool,
) -> Result<()> {
    queue!(
        terminal.backend_mut(),
        MoveTo(0, 0),
        TerminalClear(ClearType::All)
    )?;
    if purge {
        queue!(terminal.backend_mut(), TerminalClear(ClearType::Purge))?;
    }
    Ok(())
}

pub(crate) fn move_to(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    x: u16,
    y: u16,
) -> Result<()> {
    queue!(terminal.backend_mut(), MoveTo(x, y))?;
    Ok(())
}

pub(crate) fn write_terminal_line_without_newline(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    line: &Line<'_>,
    width: usize,
) -> Result<()> {
    queue!(
        terminal.backend_mut(),
        MoveToColumn(0),
        TerminalClear(ClearType::CurrentLine)
    )?;

    let mut remaining = width.saturating_sub(1);
    for span in &line.spans {
        if remaining == 0 {
            break;
        }

        let text = take_terminal_chars(span.content.as_ref(), remaining);
        if text.is_empty() {
            continue;
        }
        remaining = remaining.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        apply_terminal_style(terminal, span.style)?;
        queue!(terminal.backend_mut(), Print(text))?;
    }
    queue!(
        terminal.backend_mut(),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    Ok(())
}

fn take_terminal_chars(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in line.chars().take(width) {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

fn apply_terminal_style(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    style: Style,
) -> Result<()> {
    queue!(
        terminal.backend_mut(),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    if let Some(color) = style.fg.and_then(to_crossterm_color) {
        queue!(terminal.backend_mut(), SetForegroundColor(color))?;
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Bold))?;
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Italic))?;
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Underlined))?;
    }
    if style.add_modifier.contains(Modifier::DIM) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Dim))?;
    }
    if style.add_modifier.contains(Modifier::CROSSED_OUT) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::CrossedOut))?;
    }
    Ok(())
}

fn to_crossterm_color(color: RColor) -> Option<CTermColor> {
    match color {
        RColor::Reset => None,
        RColor::Black => Some(CTermColor::Black),
        RColor::Red => Some(CTermColor::DarkRed),
        RColor::Green => Some(CTermColor::DarkGreen),
        RColor::Yellow => Some(CTermColor::DarkYellow),
        RColor::Blue => Some(CTermColor::DarkBlue),
        RColor::Magenta => Some(CTermColor::DarkMagenta),
        RColor::Cyan => Some(CTermColor::DarkCyan),
        RColor::Gray => Some(CTermColor::Grey),
        RColor::DarkGray => Some(CTermColor::DarkGrey),
        RColor::LightRed => Some(CTermColor::Red),
        RColor::LightGreen => Some(CTermColor::Green),
        RColor::LightYellow => Some(CTermColor::Yellow),
        RColor::LightBlue => Some(CTermColor::Blue),
        RColor::LightMagenta => Some(CTermColor::Magenta),
        RColor::LightCyan => Some(CTermColor::Cyan),
        RColor::White => Some(CTermColor::White),
        RColor::Rgb(r, g, b) => Some(CTermColor::Rgb { r, g, b }),
        RColor::Indexed(index) => Some(CTermColor::AnsiValue(index)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetScrollRegion(std::ops::Range<u16>);

impl crossterm::Command for SetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[{};{}r", self.0.start, self.0.end)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SetScrollRegion is ANSI-only",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResetScrollRegion;

impl crossterm::Command for ResetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[r")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ResetScrollRegion is ANSI-only",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_viewport_fills_from_top_before_scrolling() {
        let mut viewport = HistoryViewportState::default();

        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 0,
                scroll: false
            })
        );
        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 1,
                scroll: false
            })
        );
        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 2,
                scroll: false
            })
        );
        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 2,
                scroll: true
            })
        );
    }

    #[test]
    fn history_viewport_clamps_after_resize() {
        let mut viewport = HistoryViewportState::default();
        for _ in 0..5 {
            viewport.next_insert(5);
        }

        viewport.clamp_to_height(2);

        assert_eq!(
            viewport.next_insert(2),
            Some(HistoryInsert {
                row: 1,
                scroll: true
            })
        );
    }

    #[test]
    fn viewport_growth_scrolls_history_before_panel_expands() {
        assert_eq!(
            viewport_growth_scroll(24, 3, 9, 24),
            Some(ViewportGrowth {
                previous_top: 21,
                scroll_by: 6
            })
        );
    }

    #[test]
    fn viewport_growth_does_not_scroll_when_panel_shrinks() {
        assert_eq!(viewport_growth_scroll(24, 9, 3, 24), None);
    }

    #[test]
    fn viewport_growth_does_not_scroll_sparse_history() {
        assert_eq!(viewport_growth_scroll(24, 4, 7, 5), None);
    }

    #[test]
    fn viewport_growth_scrolls_only_overflowing_sparse_history() {
        assert_eq!(
            viewport_growth_scroll(24, 4, 7, 19),
            Some(ViewportGrowth {
                previous_top: 20,
                scroll_by: 2
            })
        );
    }
}
