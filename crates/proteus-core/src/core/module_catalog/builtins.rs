use std::sync::Arc;

use anyhow::{Result, bail};

use super::{BuiltinModuleCatalog, ModuleBuildContext, PolicyBuildContext};
use crate::{
    adapters::{build_anthropic_messages_adapter, build_openai_responses_adapter},
    contracts::{
        ApprovalPolicy, ContextBuilder, HistoryCompactor, MemoryPolicy, MemoryStore, ModelAdapter,
        PatchApplier, Renderer, SearchBackend, ToolExposure, Workflow,
    },
    core::ModelConfig,
    domain::{ModuleKind, ModuleManifest, slot},
    stubs::{
        AllVisibleToolExposure, DenyAllPolicy, DynamicToolExposure, EmptyContextBuilder,
        FakeModelClient, NoCompactor, NoMemory, NoMemoryPolicy, NoWorkflow, NullPatchApplier,
        NullSearch, TextRenderer,
    },
};

pub(super) fn register_builtins(catalog: &mut BuiltinModuleCatalog) {
    // Model adapters
    catalog.register_model(
        "fake",
        manifest(
            "fake",
            ModuleKind::Model,
            &["testing", "tools"],
            "Fake model adapter for tests and local development.",
        ),
        build_fake_model_adapter,
    );
    catalog.register_model(
        "openai",
        manifest(
            "openai",
            ModuleKind::Model,
            &["responses", "tools"],
            "OpenAI Responses API adapter.",
        ),
        build_openai_model_adapter,
    );
    catalog.register_model(
        "openai_compatible",
        manifest(
            "openai_compatible",
            ModuleKind::Model,
            &["responses", "tools", "custom_base_url"],
            "OpenAI-compatible Responses API adapter.",
        ),
        build_openai_model_adapter,
    );
    catalog.register_model(
        "anthropic",
        manifest(
            "anthropic",
            ModuleKind::Model,
            &["messages", "tools"],
            "Anthropic Messages API adapter.",
        ),
        build_anthropic_model_adapter,
    );

    // Search backends
    catalog.register_module::<dyn SearchBackend>(
        slot::SEARCH,
        "null",
        manifest(
            "null",
            ModuleKind::Search,
            &["disabled"],
            "No-op search backend.",
        ),
        build_null_search,
    );
    // Memory stores
    catalog.register_module::<dyn MemoryStore>(
        slot::MEMORY,
        "none",
        manifest(
            "none",
            ModuleKind::Memory,
            &["disabled"],
            "No-op memory store.",
        ),
        build_no_memory,
    );
    // Memory policies
    catalog.register_module::<dyn MemoryPolicy>(
        slot::MEMORY_POLICY,
        "none",
        manifest(
            "none",
            ModuleKind::MemoryPolicy,
            &["disabled"],
            "No-op memory lifecycle policy.",
        ),
        build_no_memory_policy,
    );

    // Context builders
    catalog.register_module::<dyn ContextBuilder>(
        slot::CONTEXT,
        "none",
        manifest(
            "none",
            ModuleKind::Context,
            &["disabled"],
            "Empty context builder.",
        ),
        build_empty_context,
    );

    // Approval policies
    catalog.register_policy(
        "deny_all",
        manifest(
            "deny_all",
            ModuleKind::Policy,
            &["disabled", "safe_default"],
            "Deny all tool calls.",
        ),
        build_deny_all_policy,
    );

    // Patch appliers
    catalog.register_module::<dyn PatchApplier>(
        slot::PATCH,
        "null",
        manifest(
            "null",
            ModuleKind::Patch,
            &["disabled"],
            "No-op patch applier.",
        ),
        build_null_patch,
    );

    // History compactors
    catalog.register_module::<dyn HistoryCompactor>(
        slot::COMPACTOR,
        "none",
        manifest(
            "none",
            ModuleKind::Compactor,
            &["disabled"],
            "No-op request-time history compactor.",
        ),
        build_no_compactor,
    );

    // Tool exposure/selectors
    catalog.register_module::<dyn ToolExposure>(
        slot::TOOL_EXPOSURE,
        "all_visible",
        manifest(
            "all_visible",
            ModuleKind::ToolExposure,
            &["default"],
            "Expose all policy-visible tools, optionally capped by request.",
        ),
        build_all_visible_tool_exposure,
    );
    catalog.register_module::<dyn ToolExposure>(
        slot::TOOL_EXPOSURE,
        "dynamic",
        manifest(
            "dynamic",
            ModuleKind::ToolExposure,
            &["lexical", "token_savings"],
            "Expose a small lexical-ranked hot toolset from policy-visible tools.",
        ),
        build_dynamic_tool_exposure,
    );

    // Workflows
    catalog.register_module::<dyn Workflow>(
        slot::WORKFLOW,
        "none",
        manifest(
            "none",
            ModuleKind::Workflow,
            &["disabled"],
            "No-op workflow placeholder.",
        ),
        build_no_workflow,
    );

    // Renderers
    catalog.register_module::<dyn Renderer>(
        slot::RENDERER,
        "text",
        manifest(
            "text",
            ModuleKind::Renderer,
            &["plain_text"],
            "Render AgentOutput.text without decoration.",
        ),
        build_text_renderer,
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

fn build_fake_model_adapter(config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
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

fn build_openai_model_adapter(config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
    build_openai_responses_adapter(provider_config_with_stream(config)?)
}

fn build_anthropic_model_adapter(config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
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

fn build_null_search(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SearchBackend>> {
    Ok(Arc::new(NullSearch))
}

fn build_no_memory(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn MemoryStore>> {
    Ok(Arc::new(NoMemory))
}

fn build_no_memory_policy(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn MemoryPolicy>> {
    Ok(Arc::new(NoMemoryPolicy))
}

fn build_empty_context(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn ContextBuilder>> {
    Ok(Arc::new(EmptyContextBuilder))
}

fn build_deny_all_policy(_ctx: &PolicyBuildContext<'_>) -> Result<Arc<dyn ApprovalPolicy>> {
    Ok(Arc::new(DenyAllPolicy))
}

fn build_null_patch(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn PatchApplier>> {
    Ok(Arc::new(NullPatchApplier))
}

fn build_no_compactor(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn HistoryCompactor>> {
    Ok(Arc::new(NoCompactor))
}

fn build_all_visible_tool_exposure(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn ToolExposure>> {
    Ok(Arc::new(AllVisibleToolExposure))
}

fn build_dynamic_tool_exposure(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn ToolExposure>> {
    let config = ctx.config.module_config_or(
        ModuleKind::ToolExposure,
        "dynamic",
        crate::stubs::DynamicToolExposureConfig::default(),
    )?;
    Ok(Arc::new(DynamicToolExposure::new(config)))
}

fn build_no_workflow(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Workflow>> {
    Ok(Arc::new(NoWorkflow))
}

fn build_text_renderer(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Renderer>> {
    Ok(Arc::new(TextRenderer))
}
