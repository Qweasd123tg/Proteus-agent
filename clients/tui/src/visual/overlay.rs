use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::session_picker::ResumePicker;

use super::{VisualState, truncate};

pub(crate) struct VisualSurface {
    resume_picker: ResumePickerComponent,
    context_report: ContextReportComponent,
}

impl Default for VisualSurface {
    fn default() -> Self {
        Self {
            resume_picker: ResumePickerComponent,
            context_report: ContextReportComponent,
        }
    }
}

impl VisualSurface {
    pub(crate) fn render_overlay(&self, frame: &mut Frame, state: &VisualState<'_>) {
        if let Some(picker) = state.resume_picker {
            self.resume_picker.render(frame, frame.area(), picker);
            frame.set_cursor_position(Position::new(
                picker
                    .query
                    .chars()
                    .count()
                    .min(frame.area().width.saturating_sub(1) as usize) as u16,
                1,
            ));
            return;
        }
        if let Some(report) = state.context_report {
            self.context_report
                .render(frame, frame.area(), report, state.context_report_scroll);
        }
    }
}

struct ResumePickerComponent;
struct ContextReportComponent;

impl ResumePickerComponent {
    fn render(&self, frame: &mut Frame, full: Rect, picker: &ResumePicker) {
        let area = full;
        frame.render_widget(Clear, area);

        let items = picker.filtered_items();
        let list_height = area.height.saturating_sub(5) as usize;
        let selected = picker.selected.min(items.len().saturating_sub(1));
        let start = if selected >= list_height && list_height > 0 {
            selected + 1 - list_height
        } else {
            0
        };
        let end = (start + list_height).min(items.len());
        let width = area.width as usize;
        let conversation_width = width.saturating_sub(41).max(12);

        let mut body: Vec<Line<'static>> = Vec::new();
        body.push(Line::from(vec![
            Span::styled(
                "Resume a previous session",
                Style::default().fg(Color::Reset),
            ),
            Span::raw("  "),
            Span::styled("Sort: Updated", Style::default().fg(Color::DarkGray)),
        ]));
        if picker.query.is_empty() {
            body.push(Line::from(Span::styled(
                "Type to search",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            body.push(Line::from(picker.query.clone()));
        }
        body.push(Line::from(vec![
            Span::styled("  Created      ", Style::default().fg(Color::DarkGray)),
            Span::styled("Updated      ", Style::default().fg(Color::DarkGray)),
            Span::styled("Branch  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Conversation", Style::default().fg(Color::DarkGray)),
        ]));

        if items.is_empty() {
            body.push(Line::from(Span::styled(
                "  No sessions found for this workspace.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (index, item) in items[start..end].iter().enumerate() {
                let absolute_index = start + index;
                let selected_row = absolute_index == selected;
                let marker = if selected_row { "› " } else { "  " };
                let style = if selected_row {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::Reset)
                };
                body.push(Line::from(vec![
                    Span::styled(marker, style),
                    Span::styled(pad_right(&item.created, 13), style),
                    Span::styled(pad_right(&item.updated_label, 13), style),
                    Span::styled(pad_right(&item.branch, 8), style),
                    Span::styled(truncate(&item.conversation, conversation_width), style),
                ]));
            }
        }

        frame.render_widget(Paragraph::new(body), area);
    }
}

impl ContextReportComponent {
    fn render(&self, frame: &mut Frame, full: Rect, report: &str, scroll: usize) {
        frame.render_widget(Clear, full);
        let width = full.width as usize;
        let mut body = Vec::<Line<'static>>::new();
        body.push(Line::from(vec![
            Span::styled("Context Usage", Style::default().fg(Color::Reset)),
            Span::raw("  "),
            Span::styled("Esc close", Style::default().fg(Color::DarkGray)),
        ]));
        body.push(Line::raw(""));

        let content_width = width.saturating_sub(1).max(1);
        let content_height = full.height.saturating_sub(2) as usize;
        let rendered =
            crate::markdown::render_assistant_markdown(report, "", Style::default(), content_width);
        let max_scroll = rendered.len().saturating_sub(content_height);
        let start = scroll.min(max_scroll);
        body.extend(rendered.into_iter().skip(start).take(content_height));

        frame.render_widget(Paragraph::new(body), full);
    }
}

fn pad_right(input: &str, width: usize) -> String {
    let truncated = truncate(input, width);
    let padding = width.saturating_sub(truncated.chars().count());
    format!("{truncated}{}", " ".repeat(padding))
}
