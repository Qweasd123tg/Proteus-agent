use std::sync::Arc;

use anyhow::{Result, bail};

use super::{ModuleBuildContext, ModuleCatalog};
use crate::{
    adapters::{build_anthropic_messages_adapter, build_openai_responses_adapter},
    contracts::{Model, SubagentRunner},
    core::{ModelConfig, ProcessSubagentRunner},
    domain::{ModuleKind, ModuleManifest, slot},
    stubs::FakeModelClient,
};

pub(super) fn register_builtins(catalog: &mut ModuleCatalog) {
    // Model adapters
    catalog.register_model(
        "fake",
        manifest(
            "fake",
            ModuleKind::Model,
            &["testing", "tools"],
            "Фейковая модель для тестов и локальной разработки: отвечает заглушками без сети.",
        ),
        build_fake_model_adapter,
    );
    catalog.register_model(
        "openai",
        manifest(
            "openai",
            ModuleKind::Model,
            &["responses", "tools"],
            "Адаптер OpenAI Responses API.",
        ),
        build_openai_model_adapter,
    );
    catalog.register_model(
        "openai_compatible",
        manifest(
            "openai_compatible",
            ModuleKind::Model,
            &["responses", "tools", "custom_base_url"],
            "Адаптер OpenAI-совместимых Responses API (кастомный base_url в provider_config).",
        ),
        build_openai_model_adapter,
    );
    catalog.register_model(
        "anthropic",
        manifest(
            "anthropic",
            ModuleKind::Model,
            &["messages", "tools"],
            "Адаптер Anthropic Messages API.",
        ),
        build_anthropic_model_adapter,
    );

    // Subagent runners
    catalog.register_module::<dyn SubagentRunner>(
        slot::SUBAGENT,
        "process",
        manifest(
            "process",
            ModuleKind::Subagent,
            &[
                "process_isolation",
                "role_profiles",
                "parallel_spawn",
                "roles_from_config",
                "addressed_messages",
                "resumable_followup",
            ],
            "Ребёнок — отдельный process proteus server stdio со своим named config (роль = профиль); concurrent permits, bounded idle LRU, spawn/wait, адресный mailbox и resumable follow-up для collaboration surface.",
        ),
        build_process_subagent,
    );
}

fn manifest(
    id: &str,
    kind: ModuleKind,
    capabilities: &[&str],
    description: &str,
) -> ModuleManifest {
    let mut manifest = ModuleManifest::builtin(id, kind, capabilities);
    manifest.description = Some(description.to_owned());
    manifest
}

fn build_fake_model_adapter(config: &ModelConfig) -> Result<Arc<dyn Model>> {
    let client = if config.stream {
        let delay = config
            .provider_config
            .get("stream_delay_ms")
            .and_then(serde_json::Value::as_u64);
        FakeModelClient::with_streaming(delay)
    } else {
        FakeModelClient::default()
    };
    Ok(Arc::new(client))
}

fn build_openai_model_adapter(config: &ModelConfig) -> Result<Arc<dyn Model>> {
    build_openai_responses_adapter(provider_config_with_stream(config)?)
}

fn build_anthropic_model_adapter(config: &ModelConfig) -> Result<Arc<dyn Model>> {
    build_anthropic_messages_adapter(provider_config_with_stream(config)?)
}

fn provider_config_with_stream(config: &ModelConfig) -> Result<serde_json::Value> {
    let mut provider_config = match &config.provider_config {
        serde_json::Value::Null => serde_json::Map::new(),
        serde_json::Value::Object(map) => map.clone(),
        other => bail!(
            "provider_config for provider '{}' must be a JSON object, got {other}",
            config.provider
        ),
    };
    provider_config.insert("stream".to_owned(), serde_json::Value::Bool(config.stream));
    Ok(serde_json::Value::Object(provider_config))
}

fn build_process_subagent(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SubagentRunner>> {
    let config = ctx
        .config
        .module_config_value(ModuleKind::Subagent, "process");
    Ok(Arc::new(ProcessSubagentRunner::from_config(config)?))
}
