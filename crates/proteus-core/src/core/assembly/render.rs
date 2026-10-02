use super::{AssemblyCheckSeverity, AssemblyExportUse, AssemblyModuleSource, AssemblyPlan};

/// Человекочитаемый план: только решения, которые полезны перед запуском.
/// Полное описание разрешённых операций остаётся доступно в JSON.
pub fn render_assembly_plan(plan: &AssemblyPlan) -> String {
    let mut lines = Vec::new();
    lines.push(format!("План сборки v{}", plan.schema_version));
    lines.push(format!(
        "состояние: {}",
        if plan.is_valid() {
            "готов к запуску"
        } else {
            "запуск заблокирован"
        }
    ));
    lines.push(format!("профиль: {}", plan.profile));
    lines.push(format!("рабочий каталог: {}", plan.cwd.display()));
    lines.push(format!(
        "конфигурация: {}",
        plan.config_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(по умолчанию)".to_owned())
    ));
    if let Some(model) = &plan.model {
        lines.push(format!(
            "модель: {}/{} (профиль {})",
            model.provider, model.name, model.profile_id
        ));
    } else {
        lines.push("модель: не выбрана".to_owned());
    }
    lines.push(format!("режим разрешений: {:?}", plan.permission_mode));

    lines.push("слоты:".to_owned());
    for slot in plan.slots.iter().filter(|slot| slot.id != "hook") {
        let selection = match (&slot.module_id, &slot.source, &slot.component_id) {
            (Some(module_id), Some(source), Some(component_id)) => {
                format!(
                    "{module_id} [{}; процесс {component_id}]",
                    source_label(source)
                )
            }
            (Some(module_id), Some(source), None) => {
                format!("{module_id} [источник: {}]", source_label(source))
            }
            (Some(module_id), None, _) => module_id.clone(),
            (None, _, _) => "(модуль не выбран)".to_owned(),
        };
        lines.push(format!("  {}: {selection}", slot.id));
    }

    lines.push(format!(
        "обработчики hook (порядок вызова): {}",
        plan.hooks.join(" -> ")
    ));
    lines.push("запуск модулей:".to_owned());
    if plan.components.is_empty() {
        lines.push("  (нет)".to_owned());
    } else {
        for component in &plan.components {
            lines.push(format!("  {}: {}", component.id, component.command));
            for export in &component.exports {
                let use_state = match export.use_state {
                    AssemblyExportUse::Selected => "выбрана",
                    AssemblyExportUse::Included => "подключена",
                    AssemblyExportUse::Available => "доступна",
                };
                let host_access = if export.host_methods.is_empty() {
                    "методы Core: нет".to_owned()
                } else {
                    format!("методы Core: {}", export.host_methods.join(", "))
                };
                lines.push(format!(
                    "    {}/{} [{}; контракт {}; {}]",
                    export.slot, export.module_id, use_state, export.contract_version, host_access
                ));
            }
        }
    }

    lines.push("запрошенные инструменты:".to_owned());
    if plan.tools.requested.is_empty() {
        lines.push("  (нет)".to_owned());
    } else {
        lines.extend(
            plan.tools
                .requested
                .iter()
                .map(|name| format!("  - {name}")),
        );
    }

    lines.push("проверки:".to_owned());
    if plan.checks.is_empty() {
        lines.push("  ошибок нет".to_owned());
    } else {
        for check in &plan.checks {
            let level = match check.severity {
                AssemblyCheckSeverity::Warning => "предупреждение",
                AssemblyCheckSeverity::Error => "ошибка",
            };
            lines.push(format!("  {level} [{}]: {}", check.code, check.message));
        }
    }
    lines.join("\n")
}

fn source_label(source: &AssemblyModuleSource) -> &'static str {
    match source {
        AssemblyModuleSource::Builtin => "Core",
        AssemblyModuleSource::Process => "внешняя программа",
        AssemblyModuleSource::Config => "конфигурация",
        AssemblyModuleSource::Unknown => "неизвестен",
    }
}
