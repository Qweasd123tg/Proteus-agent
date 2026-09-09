use serde_json::Value;

pub(crate) struct CompactorConfig {
    pub(crate) trigger_tokens: Option<u32>,
    pub(crate) stream_max_retries: u64,
}

impl CompactorConfig {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "codex compactor config must be an object".to_owned())?;
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "trigger_tokens" | "stream_max_retries"))
        {
            return Err(format!("codex compactor config has unknown key '{key}'"));
        }
        let trigger_tokens = object
            .get("trigger_tokens")
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        "codex compactor trigger_tokens must be a positive u32".to_owned()
                    })
            })
            .transpose()?;
        // ModelProviderInfo::stream_max_retries in the pinned Codex source.
        let stream_max_retries = object
            .get("stream_max_retries")
            .map(|value| {
                value.as_u64().map(|value| value.min(100)).ok_or_else(|| {
                    "codex compactor stream_max_retries must be a non-negative integer".to_owned()
                })
            })
            .transpose()?
            .unwrap_or(5);
        Ok(Self {
            trigger_tokens,
            stream_max_retries,
        })
    }
}
