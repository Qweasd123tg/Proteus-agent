use super::{AssemblyCheckSeverity, AssemblyExportUse, AssemblyPlan};

/// Человекочитаемый план: только решения, которые полезны перед запуском.
/// Полная contract authority остаётся доступна в JSON projection.
pub fn render_assembly_plan(plan: &AssemblyPlan) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Assembly plan v{}", plan.schema_version));
    lines.push(format!(
        "status: {}",
        if plan.is_valid() { "ready" } else { "blocked" }
    ));
    lines.push(format!("profile: {}", plan.profile));
    lines.push(format!("cwd: {}", plan.cwd.display()));
    lines.push(format!(
        "config: {}",
        plan.config_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(defaults)".to_owned())
    ));
    if let Some(model) = &plan.model {
        lines.push(format!(
            "model: {}/{} (profile {})",
            model.provider, model.name, model.profile_id
        ));
    } else {
        lines.push("model: unresolved".to_owned());
    }
    lines.push(format!("permission mode: {:?}", plan.permission_mode));

    lines.push("slots:".to_owned());
    for slot in &plan.slots {
        let selection = match (&slot.module_id, &slot.source, &slot.component_id) {
            (Some(module_id), Some(source), Some(component_id)) => {
                format!("{module_id} [{source:?}, component {component_id}]")
            }
            (Some(module_id), Some(source), None) => format!("{module_id} [{source:?}]"),
            (Some(module_id), None, _) => module_id.clone(),
            (None, _, _) => "(host structural behavior)".to_owned(),
        };
        lines.push(format!("  {}: {selection}", slot.id));
    }

    lines.push("components:".to_owned());
    if plan.components.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        for component in &plan.components {
            lines.push(format!("  {}: {}", component.id, component.command));
            for export in &component.exports {
                let use_state = match export.use_state {
                    AssemblyExportUse::Selected => "selected",
                    AssemblyExportUse::Included => "included",
                    AssemblyExportUse::Available => "available",
                };
                let host_access = if export.host_methods.is_empty() {
                    "no host callbacks".to_owned()
                } else {
                    format!("host callbacks: {}", export.host_methods.join(", "))
                };
                lines.push(format!(
                    "    {}/{} [{}; {}; {}]",
                    export.slot, export.module_id, use_state, export.contract_version, host_access
                ));
            }
        }
    }

    lines.push("requested tools:".to_owned());
    if plan.tools.requested.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        lines.extend(
            plan.tools
                .requested
                .iter()
                .map(|name| format!("  - {name}")),
        );
    }

    lines.push("checks:".to_owned());
    if plan.checks.is_empty() {
        lines.push("  ok".to_owned());
    } else {
        for check in &plan.checks {
            let level = match check.severity {
                AssemblyCheckSeverity::Warning => "warning",
                AssemblyCheckSeverity::Error => "error",
            };
            lines.push(format!("  {level} [{}]: {}", check.code, check.message));
        }
    }
    lines.join("\n")
}
