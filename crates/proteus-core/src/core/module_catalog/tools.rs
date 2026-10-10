use std::{path::Path, sync::Arc};

use anyhow::{Result, bail};

use crate::{
    contracts::{ToolRegistry, ToolSource, register_provider_tools},
    core::AppConfig,
    domain::ToolSpec,
    tools::{BuiltinToolProvider, is_builtin_tool_name, register_configured_tools},
};

use super::{ModuleBuildContext, ModuleCatalog};

pub(crate) struct BuiltToolSurface {
    pub tools: ToolRegistry,
    pub mcp_servers: Vec<proteus_contracts::app_protocol::addons::AppMcpServerState>,
    /// Read-only discovery, including disabled tools. Never an execution registry.
    pub process_tool_specs: Vec<(ToolSource, ToolSpec)>,
}

impl ModuleCatalog {
    /// Builds the configured tool surface for operational inspection without
    /// exposing the host-owned structural absence implementations.
    pub fn build_tools_for_inspection(
        &self,
        config: &AppConfig,
        cwd: &Path,
    ) -> Result<ToolRegistry> {
        let addons = crate::core::agent_plugins::resolve(&config.addons, cwd);
        let context_providers = self.build_context_providers(cwd, &addons.skills)?;
        let ctx = ModuleBuildContext {
            config,
            cwd,
            context_providers: &context_providers,
        };
        self.build_tools(&ctx, &addons.skills, &addons.servers)
            .map(|surface| surface.tools)
    }

    pub(crate) fn build_tools(
        &self,
        ctx: &ModuleBuildContext<'_>,
        skills: &crate::domain::SkillRuntimeSettings,
        plugin_servers: &[crate::domain::ConfiguredMcpServerConfig],
    ) -> Result<BuiltToolSurface> {
        let mut tools = ToolRegistry::new();
        let process_tools_by_name =
            crate::process_adapters::build_process_tools(&self.process_tools, ctx.cwd, skills)?;
        let mut process_tool_specs = process_tools_by_name
            .values()
            .map(|provided| (provided.source.clone(), provided.tool.spec()))
            .collect::<Vec<_>>();
        process_tool_specs.sort_by(|(_, left), (_, right)| left.name.cmp(&right.name));
        let builtin_names = ctx
            .config
            .tools
            .enabled
            .iter()
            .filter(|name| is_builtin_tool_name(name))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(name) =
            ctx.config.tools.enabled.iter().find(|name| {
                !is_builtin_tool_name(name) && !process_tools_by_name.contains_key(*name)
            })
        {
            bail!(
                "unsupported tool: '{name}'. Configure a process Tool module that provides it or remove it from tools.enabled."
            );
        }

        let builtin_tools = BuiltinToolProvider::new(builtin_names);
        register_provider_tools(&mut tools, &builtin_tools)?;
        let mut mcp_servers = ctx.config.tools.mcp_servers.clone();
        mcp_servers.extend_from_slice(plugin_servers);
        for server in &mut mcp_servers {
            server.enabled &= !ctx
                .config
                .addons
                .disabled_mcp_servers
                .contains(&server.name);
        }
        let states = register_configured_tools(
            &mut tools,
            &ctx.config.tools.configured,
            &mcp_servers,
            ctx.cwd,
        )?;

        for name in &ctx.config.tools.enabled {
            let Some(provided) = process_tools_by_name.get(name) else {
                continue;
            };
            let spec = provided.tool.spec();
            if tools.get(&spec.name).is_some() {
                bail!(
                    "process tool '{}' conflicts with an already registered builtin/configured tool",
                    spec.name
                );
            }
            tools.register_arc(provided.source.clone(), Arc::clone(&provided.tool))?;
        }
        Ok(BuiltToolSurface {
            tools,
            mcp_servers: states,
            process_tool_specs,
        })
    }
}
