use super::*;

/// Сколько ячеек в цветной карте окна: 20 колонок × 10 рядов.
const CONTEXT_MAP_CELLS: usize = 200;
/// Сегмент карты окна: категория содержимого либо служебная зона
/// (резерв автокомпакта, свободное место).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ContextMapSegment {
    pub(super) label: String,
    pub(super) tokens: u32,
    pub(super) color: &'static str,
    pub(super) percent: f64,
}

const SEGMENT_FALLBACK_PALETTE: [&str; 6] = [
    "#6b9eff", "#5cc784", "#d8a21e", "#c08cf0", "#5cc0d8", "#ef8fb7",
];
const FREE_COLOR: &str = "#2e2e2e";
const BUFFER_COLOR: &str = "#54383d";

/// Сегменты карты окна: категории содержимого (масштабированные к фактическому
/// input — сумма локальных оценок может с ним расходиться), затем свободное
/// место и резерв автокомпакта в хвосте окна.
pub(super) fn context_map_segments(
    categories: &[ContextUsageCategory],
    used_tokens: u32,
    max_tokens: Option<u32>,
    trigger_tokens: Option<u32>,
) -> Vec<ContextMapSegment> {
    // Кэш-категории провайдера пересекаются с обычным input: на карте окна они
    // задвоили бы занятое. Их место — в панели кэша.
    let content: Vec<&ContextUsageCategory> = categories
        .iter()
        .filter(|category| category.tokens > 0 && !category.name.starts_with("provider_cache"))
        .collect();
    let content_total: u64 = content
        .iter()
        .map(|category| u64::from(category.tokens))
        .sum();

    let mut segments = Vec::new();
    for (index, category) in content.iter().enumerate() {
        let tokens = (u64::from(used_tokens) * u64::from(category.tokens))
            .checked_div(content_total)
            .unwrap_or(0) as u32;
        segments.push(ContextMapSegment {
            label: context_category_label(&category.name),
            tokens,
            color: context_category_color(&category.name, index),
            percent: 0.0,
        });
    }
    // Категорий нет (нулевые или только кэш) — рисуем занятое одним куском.
    if segments.is_empty() && used_tokens > 0 {
        segments.push(ContextMapSegment {
            label: "занято".to_owned(),
            tokens: used_tokens,
            color: SEGMENT_FALLBACK_PALETTE[0],
            percent: 0.0,
        });
    }

    if let Some(max) = max_tokens.filter(|max| *max > 0) {
        let buffer = trigger_tokens
            .filter(|trigger| *trigger < max)
            .map(|trigger| max - trigger)
            .unwrap_or(0);
        let free = max.saturating_sub(used_tokens).saturating_sub(buffer);
        if free > 0 {
            segments.push(ContextMapSegment {
                label: "свободно".to_owned(),
                tokens: free,
                color: FREE_COLOR,
                percent: 0.0,
            });
        }
        if buffer > 0 {
            segments.push(ContextMapSegment {
                label: "резерв автокомпакта".to_owned(),
                tokens: buffer,
                color: BUFFER_COLOR,
                percent: 0.0,
            });
        }
    }

    let basis = max_tokens
        .filter(|max| *max > 0)
        .map(u64::from)
        .unwrap_or_else(|| {
            segments
                .iter()
                .map(|segment| u64::from(segment.tokens))
                .sum::<u64>()
                .max(1)
        });
    for segment in &mut segments {
        segment.percent = f64::from(segment.tokens) / basis as f64 * 100.0;
    }
    segments
}

/// Распределение ячеек карты по сегментам: метод наибольших остатков, ненулевой
/// сегмент получает минимум одну ячейку (иначе тонкая категория исчезает).
pub(super) fn allocate_map_cells(tokens: &[u32], total_cells: usize) -> Vec<usize> {
    let total: u64 = tokens.iter().copied().map(u64::from).sum();
    if total == 0 || total_cells == 0 {
        return vec![0; tokens.len()];
    }

    let mut cells = Vec::with_capacity(tokens.len());
    let mut remainders = Vec::with_capacity(tokens.len());
    let mut allocated = 0usize;
    for (index, item) in tokens.iter().enumerate() {
        let exact = u64::from(*item) as f64 * total_cells as f64 / total as f64;
        let floor = exact.floor() as usize;
        cells.push(floor);
        allocated += floor;
        remainders.push((index, exact - exact.floor()));
    }
    remainders.sort_by(|left, right| right.1.total_cmp(&left.1));
    for (index, _) in remainders
        .into_iter()
        .take(total_cells.saturating_sub(allocated))
    {
        cells[index] += 1;
    }

    for index in 0..tokens.len() {
        if tokens[index] > 0 && cells[index] == 0 {
            let Some(largest) = (0..cells.len())
                .filter(|other| cells[*other] > 1)
                .max_by_key(|other| cells[*other])
            else {
                continue;
            };
            cells[largest] -= 1;
            cells[index] += 1;
        }
    }
    cells
}

pub(super) fn context_map_cell_views(segments: &[ContextMapSegment]) -> Vec<AnyView> {
    let tokens: Vec<u32> = segments.iter().map(|segment| segment.tokens).collect();
    let cells = allocate_map_cells(&tokens, CONTEXT_MAP_CELLS);
    let mut views = Vec::with_capacity(CONTEXT_MAP_CELLS);
    for (segment, count) in segments.iter().zip(cells) {
        let title = format!(
            "{} · {} · {:.0}%",
            segment.label,
            format_token_count(segment.tokens),
            segment.percent,
        );
        for _ in 0..count {
            views.push(
                view! {
                    <span
                        class="context-map-cell"
                        style=format!("background: {}", segment.color)
                        title=title.clone()
                    ></span>
                }
                .into_any(),
            );
        }
    }
    views
}

pub(super) fn context_map_legend_views(segments: &[ContextMapSegment]) -> Vec<AnyView> {
    segments
        .iter()
        .map(|segment| {
            view! {
                <div class="context-legend-row">
                    <span
                        class="context-legend-dot"
                        style=format!("background: {}", segment.color)
                    ></span>
                    <span class="context-legend-label">{segment.label.clone()}</span>
                    <code>{format_token_count(segment.tokens)}</code>
                    <code class="context-legend-percent">{format!("{:.0}%", segment.percent)}</code>
                </div>
            }
            .into_any()
        })
        .collect()
}

fn context_category_color(name: &str, index: usize) -> &'static str {
    match name {
        "instructions" => "#6b9eff",
        "messages" => "#5cc784",
        "context" => "#c08cf0",
        "tool_calls" => "#e0975c",
        "tool_results" => "#d8a21e",
        "tool_schemas" => "#5cc0d8",
        "files" => "#7fd4b2",
        "patches" => "#ef8fb7",
        _ => SEGMENT_FALLBACK_PALETTE[index % SEGMENT_FALLBACK_PALETTE.len()],
    }
}

fn context_category_label(name: &str) -> String {
    match name {
        "instructions" => "instructions".to_owned(),
        "messages" => "messages/history".to_owned(),
        "context" => "ephemeral context".to_owned(),
        "tool_calls" => "tool calls".to_owned(),
        "tool_results" => "tool results".to_owned(),
        "files" => "files".to_owned(),
        "patches" => "patches".to_owned(),
        "tool_schemas" => "tool schemas".to_owned(),
        "provider_cache_read" => "provider cache read".to_owned(),
        "provider_cache_write" => "provider cache write".to_owned(),
        other => other.replace('_', " "),
    }
}
