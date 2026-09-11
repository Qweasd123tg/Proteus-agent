use super::*;
use super::{cache::*, map::*};

fn category(name: &str, tokens: u32) -> ContextUsageCategory {
    ContextUsageCategory {
        name: name.to_owned(),
        tokens,
        source: None,
    }
}

#[test]
fn allocate_map_cells_sums_to_total_and_keeps_thin_segments_visible() {
    let cells = allocate_map_cells(&[1, 999, 0, 500], 200);

    assert_eq!(cells.iter().sum::<usize>(), 200);
    // Тонкий ненулевой сегмент виден минимум одной ячейкой.
    assert!(cells[0] >= 1);
    // Нулевой не занимает место.
    assert_eq!(cells[2], 0);
}

#[test]
fn context_map_segments_add_free_space_and_autocompact_buffer() {
    let categories = vec![category("instructions", 30), category("messages", 70)];
    let segments = context_map_segments(&categories, 100, Some(200), Some(160));

    let labels: Vec<&str> = segments
        .iter()
        .map(|segment| segment.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec![
            "instructions",
            "messages/history",
            "свободно",
            "резерв автокомпакта"
        ]
    );
    // 100 занято, до порога 160 свободно 60, резерв 200-160 = 40.
    assert_eq!(segments[2].tokens, 60);
    assert_eq!(segments[3].tokens, 40);
    // Проценты считаются от полного окна.
    assert_eq!(segments[3].percent.round() as u32, 20);
}

#[test]
fn context_map_segments_skip_provider_cache_categories() {
    let categories = vec![
        category("messages", 50),
        category("provider_cache_read", 40),
    ];
    let segments = context_map_segments(&categories, 50, None, None);

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].label, "messages/history");
    // Без известного окна проценты — от суммы сегментов.
    assert_eq!(segments[0].percent.round() as u32, 100);
}

#[test]
fn context_cache_status_tracks_cold_warming_and_hot_states() {
    assert_eq!(
        context_cache_status(100, 0, 0, Some(0)),
        ContextCacheStatus::Cold
    );
    assert_eq!(
        context_cache_status(100, 0, 80, Some(0)),
        ContextCacheStatus::Warming
    );
    assert_eq!(
        context_cache_status(100, 20, 0, Some(20)),
        ContextCacheStatus::Warming
    );
    assert_eq!(
        context_cache_status(100, 75, 0, Some(75)),
        ContextCacheStatus::Hot
    );
}

#[test]
fn context_cache_view_model_handles_missing_usage() {
    let cache = context_cache_view_model(None);

    assert_eq!(cache.status, "n/a");
    assert_eq!(cache.input_tokens, "n/a");
    assert_eq!(cache.hit_rate, "n/a");
    assert_eq!(cache.hit_percent, 0);
}

#[test]
fn context_cache_view_model_formats_provider_usage() {
    let usage = ContextUsageSnapshot {
        model_provider: "openai".to_owned(),
        model_name: "gpt-test".to_owned(),
        phase: Some("execute".to_owned()),
        estimated_input_tokens: 100,
        max_input_tokens: Some(1000),
        compaction_trigger_tokens: None,
        categories: Vec::new(),
        actual: Some(ContextActualUsage {
            input_tokens: 2000,
            output_tokens: 10,
            cached_input_tokens: Some(1500),
            cache_creation_input_tokens: Some(0),
            reasoning_output_tokens: None,
        }),
        source: "mixed".to_owned(),
        turn_id: None,
        timestamp_ms: None,
    };

    let cache = context_cache_view_model(Some(&usage));

    assert_eq!(cache.status, "hot");
    assert_eq!(cache.input_tokens, "2k");
    assert_eq!(cache.cached_input_tokens, "1.5k");
    assert_eq!(cache.cache_creation_input_tokens, "0");
    assert_eq!(cache.hit_rate, "75%");
    assert_eq!(cache.hit_percent, 75);
}
