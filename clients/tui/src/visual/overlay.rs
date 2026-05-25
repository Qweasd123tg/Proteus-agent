use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{config_summary::ConfigSummary, session_picker::ResumePicker};

use super::{VisualState, truncate};

pub(crate) struct VisualSurface {
    resume_picker: ResumePickerComponent,
    config_summary: ConfigSummaryComponent,
    context_report: ContextReportComponent,
}

impl Default for VisualSurface {
    fn default() -> Self {
        Self {
            resume_picker: ResumePickerComponent,
            config_summary: ConfigSummaryComponent,
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
        if let Some(summary) = state.config_summary {
            self.config_summary
                .render(frame, frame.area(), summary, state.config_summary_scroll);
            return;
        }
        if let Some(report) = state.context_report {
            self.context_report
                .render(frame, frame.area(), report, state.context_report_scroll);
        }
    }
}

struct ResumePickerComponent;
struct ConfigSummaryComponent;
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

impl ConfigSummaryComponent {
    fn render(&self, frame: &mut Frame, full: Rect, summary: &ConfigSummary, scroll: usize) {
        frame.render_widget(Clear, full);
        let body = config_summary_overlay_lines(summary, full.width as usize, full.height, scroll);
        frame.render_widget(Paragraph::new(body), full);
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

fn config_summary_lines(summary: &ConfigSummary, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.extend(section("Launch"));
    lines.push(kv_line("profile", &summary.profile, width));
    lines.push(kv_line("model", &summary.model, width));
    lines.push(kv_line("mode", &summary.permission_mode, width));
    lines.push(kv_line("cwd", &summary.cwd, width));
    lines.push(kv_line("config path", &summary.config_path, width));
    if !summary.config_files.is_empty() {
        lines.push(Line::from(Span::styled(
            "  config files",
            Style::default().fg(Color::DarkGray),
        )));
        for file in &summary.config_files {
            lines.push(Line::from(vec![
                Span::styled("    - ", Style::default().fg(Color::DarkGray)),
                Span::raw(truncate(file, width.saturating_sub(6).max(1))),
            ]));
        }
    }

    lines.push(Line::raw(""));
    lines.extend(section("Modules"));
    if summary.modules.is_empty() {
        lines.push(empty_line("no modules reported"));
    } else {
        let left_width = 18;
        let value_width = width.saturating_sub(left_width + 4).max(1);
        for module in &summary.modules {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    pad_right(&module.slot, left_width),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(truncate(&module.id, value_width)),
            ]));
        }
    }

    lines.push(Line::raw(""));
    lines.extend(section("Enabled Tools"));
    if summary.enabled_tools.is_empty() {
        lines.push(empty_line("none"));
    } else {
        lines.push(Line::raw(truncate(
            &format!("  {}", summary.enabled_tools.join(", ")),
            width,
        )));
    }

    lines.push(Line::raw(""));
    lines.extend(section("Registered Tools"));
    if summary.registered_tools.is_empty() {
        lines.push(empty_line("none"));
    } else {
        let name_width = 18;
        let safety_width = 12;
        let source_width = 22;
        let desc_width = width
            .saturating_sub(name_width + safety_width + source_width + 8)
            .max(12);
        lines.push(table_header(&[
            ("Tool", name_width),
            ("Safety", safety_width),
            ("Source", source_width),
            ("Description", desc_width),
        ]));
        for tool in &summary.registered_tools {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    pad_right(&tool.name, name_width),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(pad_right(&tool.safety, safety_width)),
                Span::styled(
                    pad_right(&tool.source, source_width),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(truncate(&tool.description, desc_width)),
            ]));
        }
    }

    lines.push(Line::raw(""));
    lines.extend(section("Plugins"));
    if summary.plugins.is_empty() {
        lines.push(empty_line("none found"));
    } else {
        let name_width = 22;
        let version_width = 9;
        let status_width = 12;
        let desc_width = width
            .saturating_sub(name_width + version_width + status_width + 8)
            .max(12);
        lines.push(table_header(&[
            ("Plugin", name_width),
            ("Version", version_width),
            ("Status", status_width),
            ("Description", desc_width),
        ]));
        for plugin in &summary.plugins {
            let status_style = if plugin.status == "loaded" {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    pad_right(&plugin.name, name_width),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(pad_right(&plugin.version, version_width)),
                Span::styled(pad_right(&plugin.status, status_width), status_style),
                Span::raw(truncate(&plugin.description, desc_width)),
            ]));
        }
    }

    if summary.modules.is_empty()
        && summary.registered_tools.is_empty()
        && summary.plugins.is_empty()
        && !summary.fallback_text.is_empty()
    {
        lines.push(Line::raw(""));
        lines.extend(section("Raw"));
        for line in summary.fallback_text.lines() {
            lines.push(Line::raw(truncate(line, width)));
        }
    }

    lines
}

fn config_summary_overlay_lines(
    summary: &ConfigSummary,
    width: usize,
    height: u16,
    scroll: usize,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }

    let content_width = width.saturating_sub(1).max(1);
    let content_height = height.saturating_sub(2) as usize;
    let content = config_summary_lines(summary, content_width);
    let max_scroll = content.len().saturating_sub(content_height);
    let start = scroll.min(max_scroll);
    let end = if content_height == 0 {
        start
    } else {
        (start + content_height).min(content.len())
    };

    let mut lines = Vec::with_capacity(height as usize);
    lines.push(config_summary_header_line(
        width,
        start,
        end,
        content.len(),
        max_scroll,
    ));
    if height > 1 {
        lines.push(Line::raw(""));
    }
    if content_height > 0 {
        lines.extend(content.into_iter().skip(start).take(content_height));
    }
    lines
}

fn config_summary_header_line(
    width: usize,
    start: usize,
    end: usize,
    total: usize,
    max_scroll: usize,
) -> Line<'static> {
    let position = if total == 0 {
        "0 / 0".to_owned()
    } else if max_scroll == 0 {
        format!("{total} lines")
    } else {
        format!("{}-{} / {}", start + 1, end, total)
    };
    let hint = format!("Esc close · Up/Down scroll · {position}");
    let title = "Active Configuration";
    let title_len = title.chars().count();
    if width <= title_len + 2 {
        return Line::from(Span::styled(
            truncate(title, width.max(1)),
            Style::default().fg(Color::Reset),
        ));
    }
    let gap = width
        .saturating_sub(title.chars().count() + hint.chars().count())
        .max(2);
    let hint_width = width.saturating_sub(title_len + gap).max(1);
    Line::from(vec![
        Span::styled(title.to_owned(), Style::default().fg(Color::Reset)),
        Span::raw(" ".repeat(gap)),
        Span::styled(
            truncate(&hint, hint_width),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

fn section(title: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        title.to_owned(),
        Style::default().fg(Color::Yellow),
    ))]
}

fn kv_line(key: &str, value: &str, width: usize) -> Line<'static> {
    let key_width = 13;
    Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            pad_right(key, key_width),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(truncate(value, width.saturating_sub(key_width + 2).max(1))),
    ])
}

fn table_header(columns: &[(&str, usize)]) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", Style::default())];
    for (label, width) in columns {
        spans.push(Span::styled(
            pad_right(label, *width),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn empty_line(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(text.to_owned(), Style::default().fg(Color::DarkGray)),
    ])
}

fn pad_right(input: &str, width: usize) -> String {
    let truncated = truncate(input, width);
    let padding = width.saturating_sub(truncated.chars().count());
    format!("{truncated}{}", " ".repeat(padding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_summary::{ConfigModule, ConfigPlugin, ConfigTool};

    #[test]
    fn config_summary_lines_render_core_sections() {
        let summary = sample_config_summary();

        let rendered = config_summary_lines(&summary, 100)
            .into_iter()
            .map(line_to_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Launch"));
        assert!(rendered.contains("Modules"));
        assert!(rendered.contains("Registered Tools"));
        assert!(rendered.contains("file-tools"));
    }

    #[test]
    fn config_summary_overlay_keeps_header_fixed_while_scrolling() {
        let summary = sample_config_summary();

        let top = config_summary_overlay_lines(&summary, 100, 8, 0)
            .into_iter()
            .map(line_to_text)
            .collect::<Vec<_>>();
        let scrolled = config_summary_overlay_lines(&summary, 100, 8, 4)
            .into_iter()
            .map(line_to_text)
            .collect::<Vec<_>>();

        assert!(top[0].starts_with("Active Configuration"));
        assert!(scrolled[0].starts_with("Active Configuration"));
        assert_ne!(top[2], scrolled[2]);
    }

    fn sample_config_summary() -> ConfigSummary {
        ConfigSummary {
            config_path: "/tmp/configs".to_owned(),
            config_files: vec!["/tmp/configs/10-coding.toml".to_owned()],
            cwd: "/repo".to_owned(),
            profile: "claude-pack-local".to_owned(),
            model: "anthropic/deepseek-v4-pro".to_owned(),
            permission_mode: "Normal".to_owned(),
            modules: vec![ConfigModule {
                slot: "workflow".to_owned(),
                id: "claude.explore_edit_verify".to_owned(),
            }],
            enabled_tools: vec!["Read".to_owned(), "Write".to_owned()],
            registered_tools: vec![ConfigTool {
                name: "Write".to_owned(),
                source: "dynamic:plugin:dylib".to_owned(),
                safety: "WritesFiles".to_owned(),
                description: "Create files".to_owned(),
            }],
            plugins: vec![ConfigPlugin {
                name: "file-tools".to_owned(),
                version: "0.1.0".to_owned(),
                status: "loaded".to_owned(),
                description: "Basic file tools".to_owned(),
            }],
            fallback_text: String::new(),
        }
    }

    fn line_to_text(line: Line<'static>) -> String {
        line.spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>()
    }
}
