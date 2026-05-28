use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

fn elapsed_since_start() -> Duration {
    PROCESS_START.get_or_init(Instant::now).elapsed()
}

pub(crate) fn shimmer_spans(text: &str) -> Vec<Span<'static>> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return Vec::new();
    }

    let elapsed = elapsed_since_start().as_secs_f32();
    let sweep = (elapsed % 1.8) / 1.8;
    let center = sweep * (chars.len() as f32 + 8.0) - 4.0;

    chars
        .into_iter()
        .enumerate()
        .map(|(index, ch)| {
            let distance = ((index as f32) - center).abs();
            let intensity = (1.0 - distance / 4.0).clamp(0.0, 1.0);
            Span::styled(ch.to_string(), shimmer_style(intensity))
        })
        .collect()
}

fn shimmer_style(intensity: f32) -> Style {
    let (r, g, b) = blend_rgb((90, 120, 140), (245, 255, 255), intensity);
    let style = Style::default().fg(Color::Rgb(r, g, b));
    if intensity > 0.55 {
        style.add_modifier(Modifier::BOLD)
    } else if intensity < 0.12 {
        style.add_modifier(Modifier::DIM)
    } else {
        style
    }
}

fn blend_rgb(base: (u8, u8, u8), highlight: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        lerp_u8(base.0, highlight.0, t),
        lerp_u8(base.1, highlight.1, t),
        lerp_u8(base.2, highlight.2, t),
    )
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

pub(crate) fn running_tool_marker() -> &'static str {
    "●"
}

pub(crate) fn running_tool_marker_style() -> Style {
    let elapsed = elapsed_since_start();
    let bright = (elapsed.as_millis() / 520).is_multiple_of(2);
    if bright {
        Style::default()
            .fg(Color::Rgb(255, 149, 0))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

pub(crate) fn status_marker_style() -> Style {
    let elapsed = elapsed_since_start();
    let bright = (elapsed.as_millis() / 620).is_multiple_of(2);
    if bright {
        Style::default()
            .fg(Color::Rgb(140, 220, 255))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn shimmer_preserves_visible_text() {
        let rendered = shimmer_spans("responding")
            .into_iter()
            .map(|span| span.content.to_string())
            .collect::<String>();

        assert_eq!(rendered, "responding");
    }

    #[test]
    fn running_tool_marker_uses_activity_color() {
        let style = running_tool_marker_style();

        assert!(matches!(style.fg, Some(Color::Rgb(255, 149, 0)) | None));
    }

    #[test]
    fn running_tool_marker_keeps_shape_stable() {
        assert_eq!(running_tool_marker(), "●");
    }
}
