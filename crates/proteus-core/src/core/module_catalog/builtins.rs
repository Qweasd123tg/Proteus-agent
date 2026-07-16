use std::sync::Arc;

use anyhow::{Result, bail};

use super::{BuiltinModuleCatalog, ModuleBuildContext, PolicyBuildContext};
use crate::{
    adapters::{build_anthropic_messages_adapter, build_openai_responses_adapter},
    contracts::{
        ApprovalPolicy, ContextBuilder, HistoryCompactor, MemoryStore, ModelAdapter, PatchApplier,
        Renderer, SearchBackend, SubagentRunner, ToolExposure, Workflow,
    },
    core::{ModelConfig, ProcessSubagentRunner, SequentialSubagentRunner},
    domain::{ModuleKind, ModuleManifest, slot},
    stubs::{
        AllVisibleToolExposure, DenyAllPolicy, EmptyContextBuilder, FakeModelClient, NoCompactor,
        NoMemory, NoSubagent, NoWorkflow, NullPatchApplier, NullSearch, TextRenderer,
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

    // Search backends
    catalog.register_module::<dyn SearchBackend>(
        slot::SEARCH,
        "null",
        manifest(
            "null",
            ModuleKind::Search,
            &["disabled"],
            "Поиск отключён: всегда возвращает пустой результат.",
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
            "Память отключена: ничего не сохраняет и не вспоминает.",
        ),
        build_no_memory,
    );
    // Context builders
    catalog.register_module::<dyn ContextBuilder>(
        slot::CONTEXT,
        "none",
        manifest(
            "none",
            ModuleKind::Context,
            &["disabled"],
            "Не добавляет контекст: в модель уходит только задача и история.",
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
            "Запрещает все tool-вызовы. Безопасный дефолт, пока не выбрана policy.",
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
            "Патчи отключены: apply возвращает неуспех с пометкой disabled.",
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
            "Без компакции: история уходит в модель как есть.",
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
            "Показывает модели все policy-видимые tools (опциональный лимит из запроса workflow).",
        ),
        build_all_visible_tool_exposure,
    );
    // Subagent runners
    catalog.register_module::<dyn SubagentRunner>(
        slot::SUBAGENT,
        "none",
        manifest(
            "none",
            ModuleKind::Subagent,
            &["disabled"],
            "Делегирование выключено: ролей нет, run возвращает ошибку.",
        ),
        build_no_subagent,
    );
    catalog.register_module::<dyn SubagentRunner>(
        slot::SUBAGENT,
        "sequential",
        manifest(
            "sequential",
            ModuleKind::Subagent,
            &["sequential", "parallel_spawn", "roles_from_config"],
            "Дочерний агентский цикл in-process: роли и лимиты из module_config.subagent.sequential; spawn/wait для конкурентных parallel_safe-ролей.",
        ),
        build_sequential_subagent,
    );
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
            ],
            "Ребёнок — отдельный процесс proteus server stdio со своим named config (роль = профиль); concurrent permits на роль, глобальный bounded idle LRU pool и spawn/wait для parallel_safe-ролей; настройки в module_config.subagent.process.",
        ),
        build_process_subagent,
    );

    // Workflows
    catalog.register_module::<dyn Workflow>(
        slot::WORKFLOW,
        "none",
        manifest(
            "none",
            ModuleKind::Workflow,
            &["disabled"],
            "Заглушка: вместо запуска агента отвечает подсказкой выбрать workflow-плагин.",
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
            "Выводит текст ответа без оформления.",
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

fn build_no_subagent(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SubagentRunner>> {
    Ok(Arc::new(NoSubagent))
}

fn build_sequential_subagent(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SubagentRunner>> {
    let config = ctx
        .config
        .module_config_value(ModuleKind::Subagent, "sequential");
    Ok(Arc::new(SequentialSubagentRunner::from_config_with_cwd(
        config, ctx.cwd,
    )?))
}

fn build_process_subagent(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SubagentRunner>> {
    let config = ctx
        .config
        .module_config_value(ModuleKind::Subagent, "process");
    Ok(Arc::new(ProcessSubagentRunner::from_config(config)?))
}

fn build_no_workflow(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Workflow>> {
    Ok(Arc::new(NoWorkflow))
}

fn build_text_renderer(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Renderer>> {
    Ok(Arc::new(TextRenderer))
}
