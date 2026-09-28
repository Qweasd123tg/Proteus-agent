use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ContextCacheViewModel {
    pub(super) status: String,
    pub(super) status_detail: String,
    pub(super) badge_class: String,
    pub(super) input_tokens: String,
    pub(super) cached_input_tokens: String,
    pub(super) cache_creation_input_tokens: String,
    pub(super) hit_rate: String,
    pub(super) hit_title: String,
    pub(super) hit_percent: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContextCacheStatus {
    Unavailable,
    Cold,
    Warming,
    Hot,
}

impl ContextCacheStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "n/a",
            Self::Cold => "cold",
            Self::Warming => "warming",
            Self::Hot => "hot",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Unavailable => "provider usage missing",
            Self::Cold => "no cache read",
            Self::Warming => "cache warming",
            Self::Hot => "cache read active",
        }
    }

    fn badge_class(self) -> &'static str {
        match self {
            Self::Unavailable | Self::Cold => "status-badge idle",
            Self::Warming => "status-badge disconnected",
            Self::Hot => "status-badge completed",
        }
    }
}

pub(super) fn context_cache_view_model(
    usage: Option<&ContextUsageSnapshot>,
) -> ContextCacheViewModel {
    let Some(actual) = usage.and_then(|usage| usage.actual.as_ref()) else {
        return ContextCacheViewModel::from_values(ContextCacheStatus::Unavailable, 0, 0, 0, None);
    };
    let input_tokens = actual.input_tokens;
    let cached_input_tokens = actual.cached_input_tokens.unwrap_or(0);
    let cache_creation_input_tokens = actual.cache_creation_input_tokens.unwrap_or(0);
    let hit_percent = context_cache_hit_percent(input_tokens, cached_input_tokens);
    let status = context_cache_status(
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        hit_percent,
    );
    ContextCacheViewModel::from_values(
        status,
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        hit_percent,
    )
}

impl ContextCacheViewModel {
    fn from_values(
        status: ContextCacheStatus,
        input_tokens: u32,
        cached_input_tokens: u32,
        cache_creation_input_tokens: u32,
        hit_percent: Option<u32>,
    ) -> Self {
        let hit_rate = hit_percent
            .map(|percent| format!("{percent}%"))
            .unwrap_or_else(|| "n/a".to_owned());
        let hit_title = if input_tokens == 0 {
            "no provider input usage".to_owned()
        } else {
            format!(
                "{} cached / {} input",
                format_token_count(cached_input_tokens),
                format_token_count(input_tokens)
            )
        };
        Self {
            status: status.label().to_owned(),
            status_detail: status.detail().to_owned(),
            badge_class: status.badge_class().to_owned(),
            input_tokens: optional_token_count(
                input_tokens,
                status != ContextCacheStatus::Unavailable,
            ),
            cached_input_tokens: optional_token_count(
                cached_input_tokens,
                status != ContextCacheStatus::Unavailable,
            ),
            cache_creation_input_tokens: optional_token_count(
                cache_creation_input_tokens,
                status != ContextCacheStatus::Unavailable,
            ),
            hit_rate,
            hit_title,
            hit_percent: hit_percent.unwrap_or(0),
        }
    }
}

pub(super) fn context_cache_status(
    input_tokens: u32,
    cached_input_tokens: u32,
    cache_creation_input_tokens: u32,
    hit_percent: Option<u32>,
) -> ContextCacheStatus {
    if input_tokens == 0 {
        return ContextCacheStatus::Unavailable;
    }
    if cached_input_tokens == 0 && cache_creation_input_tokens == 0 {
        return ContextCacheStatus::Cold;
    }
    if hit_percent.is_some_and(|percent| percent >= 50) {
        ContextCacheStatus::Hot
    } else {
        ContextCacheStatus::Warming
    }
}

fn context_cache_hit_percent(input_tokens: u32, cached_input_tokens: u32) -> Option<u32> {
    if input_tokens == 0 {
        return None;
    }
    Some(
        ((f64::from(cached_input_tokens) / f64::from(input_tokens)) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u32,
    )
}

fn optional_token_count(tokens: u32, available: bool) -> String {
    if available {
        format_token_count(tokens)
    } else {
        "n/a".to_owned()
    }
}
