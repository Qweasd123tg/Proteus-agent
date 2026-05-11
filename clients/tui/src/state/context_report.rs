use std::collections::BTreeMap;

use agent_contracts::domain::{TokenUsageSnapshot, TokenUsageSource};

#[derive(Debug, Clone, Default)]
pub(super) struct UsageTotals {
    pub(super) requests: u32,
    pub(super) estimated_input_tokens: u32,
    pub(super) provider_reports: u32,
    pub(super) provider_input_tokens: u32,
    pub(super) provider_output_tokens: u32,
    pub(super) cached_input_tokens: u32,
    pub(super) cache_creation_input_tokens: u32,
    pub(super) reasoning_output_tokens: u32,
    pub(super) categories: BTreeMap<String, u32>,
}

impl UsageTotals {
    pub(super) fn add_snapshot(&mut self, usage: &TokenUsageSnapshot) {
        self.requests = self.requests.saturating_add(1);
        self.estimated_input_tokens = self
            .estimated_input_tokens
            .saturating_add(usage.estimated_input_tokens);
        for category in &usage.categories {
            let entry = self.categories.entry(category.name.clone()).or_default();
            *entry = entry.saturating_add(category.tokens);
        }
        if let Some(actual) = &usage.actual {
            self.provider_reports = self.provider_reports.saturating_add(1);
            self.provider_input_tokens = self
                .provider_input_tokens
                .saturating_add(actual.input_tokens);
            self.provider_output_tokens = self
                .provider_output_tokens
                .saturating_add(actual.output_tokens);
            self.cached_input_tokens = self
                .cached_input_tokens
                .saturating_add(actual.cached_input_tokens.unwrap_or_default());
            self.cache_creation_input_tokens = self
                .cache_creation_input_tokens
                .saturating_add(actual.cache_creation_input_tokens.unwrap_or_default());
            self.reasoning_output_tokens = self
                .reasoning_output_tokens
                .saturating_add(actual.reasoning_output_tokens.unwrap_or_default());
        }
    }
}

pub(super) fn provider_usage_line(actual: &agent_contracts::model_standard::TokenUsage) -> String {
    let total = actual.input_tokens + actual.output_tokens;
    let mut line = format!(
        "{} input / {} output / {} total",
        format_tokens(actual.input_tokens),
        format_tokens(actual.output_tokens),
        format_tokens(total)
    );
    let mut details = Vec::new();
    if let Some(tokens) = actual.cached_input_tokens {
        details.push(format!("cache read {}", format_tokens(tokens)));
    }
    if let Some(tokens) = actual.cache_creation_input_tokens {
        details.push(format!("cache write {}", format_tokens(tokens)));
    }
    if let Some(tokens) = actual.reasoning_output_tokens {
        details.push(format!("reasoning {}", format_tokens(tokens)));
    }
    if !details.is_empty() {
        line.push_str(" · ");
        line.push_str(&details.join(" · "));
    }
    line
}

pub(super) fn append_context_visual_summary(
    lines: &mut Vec<String>,
    usage: &TokenUsageSnapshot,
    source: &str,
) {
    lines.push(String::new());
    lines.push("Карта контекста".to_owned());

    let model = format!("{}/{}", usage.model.provider, usage.model.model);
    let (window, inferred_window) = usage
        .max_input_tokens
        .map(|window| (window, false))
        .unwrap_or_else(|| (inferred_context_window(usage), true));
    let used = usage.estimated_input_tokens.min(window);
    let free = window.saturating_sub(used);
    let percent = percent_of(used, window);
    let total_cells = 200usize;
    let row_cells = 20usize;
    let used_cells = proportional_cells(used, window, total_cells).min(total_cells);
    let free_cells = total_cells.saturating_sub(used_cells);
    let mut cells = Vec::with_capacity(total_cells);
    let category_slices = context_category_slices(usage, used_cells);
    for slice in &category_slices {
        cells.extend(std::iter::repeat_n(slice.glyph, slice.cells));
    }
    cells.extend(std::iter::repeat_n('□', free_cells));
    cells.resize(total_cells, '□');

    let window_label = if inferred_window {
        format!("{} inferred context", format_tokens(window))
    } else {
        format!("{} context", format_tokens(window))
    };
    let mut labels = vec![
        format!("{model} ({window_label})"),
        format!("source: {source}"),
        format!(
            "{} / {} tokens ({percent:.1}%)",
            format_tokens(used),
            format_tokens(window)
        ),
        "Estimated usage by category".to_owned(),
    ];
    for slice in &category_slices {
        labels.push(format!(
            "{} {}: {} tokens ({:.1}%)",
            slice.glyph,
            slice.label,
            format_tokens(slice.tokens),
            percent_of(slice.tokens, usage.estimated_input_tokens)
        ));
    }
    let cached_tokens = usage
        .actual
        .as_ref()
        .and_then(|actual| actual.cached_input_tokens)
        .unwrap_or_default()
        .min(used);
    if cached_tokens > 0 {
        labels.push(format!(
            "◉ Cache read: {} tokens",
            format_tokens(cached_tokens)
        ));
    }
    labels.push(format!(
        "□ Free space: {} ({:.1}%)",
        format_tokens(free),
        percent_of(free, window)
    ));
    if inferred_window {
        labels.push("context window inferred locally".to_owned());
    }

    for (row, chunk) in cells.chunks(row_cells).enumerate() {
        let graph = chunk
            .iter()
            .map(char::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(label) = labels.get(row) {
            lines.push(format!("{graph}   {label}"));
        } else {
            lines.push(graph);
        }
    }
}

#[derive(Debug)]
struct ContextCategorySlice {
    label: String,
    tokens: u32,
    glyph: char,
    cells: usize,
}

fn context_category_slices(
    usage: &TokenUsageSnapshot,
    used_cells: usize,
) -> Vec<ContextCategorySlice> {
    if used_cells == 0 {
        return Vec::new();
    }
    let mut raw = usage
        .categories
        .iter()
        .filter(|category| category.tokens > 0)
        .map(|category| {
            (
                category_label(&category.name),
                category.tokens,
                context_category_glyph(&category.name),
            )
        })
        .collect::<Vec<_>>();
    if raw.is_empty() {
        raw.push((
            "Input estimate".to_owned(),
            usage.estimated_input_tokens,
            '◆',
        ));
    }
    let total_tokens = raw
        .iter()
        .fold(0_u32, |total, (_, tokens, _)| total.saturating_add(*tokens))
        .max(1);
    let mut slices = raw
        .into_iter()
        .enumerate()
        .map(|(index, (label, tokens, glyph))| {
            let scaled = tokens as u128 * used_cells as u128;
            let cells = (scaled / total_tokens as u128) as usize;
            let remainder = scaled % total_tokens as u128;
            (
                index,
                remainder,
                ContextCategorySlice {
                    label,
                    tokens,
                    glyph,
                    cells,
                },
            )
        })
        .collect::<Vec<_>>();
    let assigned = slices
        .iter()
        .map(|(_, _, slice)| slice.cells)
        .sum::<usize>();
    let mut remaining = used_cells.saturating_sub(assigned);
    slices.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (_, _, slice) in &mut slices {
        if remaining == 0 {
            break;
        }
        slice.cells += 1;
        remaining -= 1;
    }
    slices.sort_by_key(|(index, _, _)| *index);
    slices
        .into_iter()
        .map(|(_, _, slice)| slice)
        .filter(|slice| slice.cells > 0)
        .collect()
}

fn context_category_glyph(name: &str) -> char {
    match name {
        "instructions" | "system_prompt" => '◆',
        "messages" => '●',
        "context" | "skills" => '▲',
        "tool_results" => '■',
        "tool_schemas" | "system_tools" => '⬟',
        _ => '◈',
    }
}

fn inferred_context_window(usage: &TokenUsageSnapshot) -> u32 {
    let model = usage.model.model.to_ascii_lowercase();
    let provider = usage.model.provider.to_ascii_lowercase();
    let known_window = if model.contains("1m")
        || model.contains("1000k")
        || model.contains("1000000")
        || model.contains("opus-4-7")
        || model.contains("opus-4.7")
    {
        1_000_000
    } else if provider.contains("anthropic") || model.contains("claude") {
        200_000
    } else if model.contains("deepseek") {
        128_000
    } else {
        200_000
    };
    known_window.max(next_visual_window(usage.estimated_input_tokens))
}

fn next_visual_window(tokens: u32) -> u32 {
    if tokens <= 128_000 {
        128_000
    } else if tokens <= 200_000 {
        200_000
    } else if tokens <= 1_000_000 {
        1_000_000
    } else {
        tokens
            .saturating_add(999_999)
            .saturating_div(1_000_000)
            .saturating_mul(1_000_000)
    }
}

pub(super) fn append_usage_totals_section(
    lines: &mut Vec<String>,
    title: &str,
    totals: &UsageTotals,
) {
    lines.push(String::new());
    lines.push(title.to_owned());
    if totals.requests == 0 {
        lines.push("no requests yet".to_owned());
        append_usage_totals_note(lines, title);
        return;
    }

    lines.push(format!("requests: {}", totals.requests));
    lines.push(format!(
        "estimated input: {}",
        format_tokens(totals.estimated_input_tokens)
    ));
    lines.push(format!("provider usage: {}", provider_totals_line(totals)));

    if totals.categories.is_empty() {
        append_usage_totals_note(lines, title);
        return;
    }
    lines.extend([
        "| Category | Tokens | Share |".to_owned(),
        "| --- | ---: | ---: |".to_owned(),
    ]);
    for (name, tokens) in &totals.categories {
        let share = if totals.estimated_input_tokens == 0 {
            0.0
        } else {
            *tokens as f64 * 100.0 / totals.estimated_input_tokens as f64
        };
        lines.push(format!(
            "| {} | {} | {:.1}% |",
            category_label(name),
            format_tokens(*tokens),
            share
        ));
    }
    append_usage_totals_note(lines, title);
}

fn append_usage_totals_note(lines: &mut Vec<String>, title: &str) {
    let note = match title {
        "Current turn totals" => {
            "Пояснение: это сумма model requests внутри текущего turn, включая повторные запросы после tool calls."
        }
        "Session totals" => {
            "Пояснение: это сумма usage events сессии, включая восстановленные события после `/resume` и новые запросы в этом клиенте."
        }
        _ => return,
    };
    lines.push(String::new());
    lines.push(note.to_owned());
}

fn provider_totals_line(totals: &UsageTotals) -> String {
    if totals.provider_reports == 0 {
        return "not reported by provider".to_owned();
    }
    let total = totals
        .provider_input_tokens
        .saturating_add(totals.provider_output_tokens);
    let mut line = format!(
        "{} input / {} output / {} total across {} request(s)",
        format_tokens(totals.provider_input_tokens),
        format_tokens(totals.provider_output_tokens),
        format_tokens(total),
        totals.provider_reports
    );
    let mut details = Vec::new();
    if totals.cached_input_tokens > 0 {
        details.push(format!(
            "cache read {}",
            format_tokens(totals.cached_input_tokens)
        ));
    }
    if totals.cache_creation_input_tokens > 0 {
        details.push(format!(
            "cache write {}",
            format_tokens(totals.cache_creation_input_tokens)
        ));
    }
    if totals.reasoning_output_tokens > 0 {
        details.push(format!(
            "reasoning {}",
            format_tokens(totals.reasoning_output_tokens)
        ));
    }
    if !details.is_empty() {
        line.push_str(" · ");
        line.push_str(&details.join(" · "));
    }
    line
}

pub(super) fn usage_source_label(source: TokenUsageSource) -> &'static str {
    match source {
        TokenUsageSource::Estimated => "estimated only",
        TokenUsageSource::Provider => "provider reported",
        TokenUsageSource::Mixed => "provider totals + estimated categories",
        _ => "unknown",
    }
}

pub(super) fn percent_of(value: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 * 100.0 / total as f64
    }
}

fn proportional_cells(value: u32, total: u32, width: usize) -> usize {
    if value == 0 || total == 0 || width == 0 {
        return 0;
    }
    let rounded = ((value.min(total) as u64 * width as u64) + (total as u64 / 2)) / total as u64;
    (rounded as usize).clamp(1, width)
}

pub(super) fn usage_bar(used: u32, total: u32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = if total == 0 {
        0
    } else {
        ((used.min(total) as usize * width) + (total as usize / 2)) / total as usize
    }
    .min(width);
    format!("[{}{}]", "#".repeat(filled), ".".repeat(width - filled))
}

pub(super) fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.saturating_add(3) / 4
}

pub(super) fn format_tokens(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}m", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

pub(super) fn category_label(name: &str) -> String {
    match name {
        "instructions" => "Instructions",
        "messages" => "Messages",
        "context" => "Context",
        "tool_results" => "Tool results",
        "files" => "Files",
        "tool_schemas" => "Tool schemas",
        other => other,
    }
    .to_owned()
}
