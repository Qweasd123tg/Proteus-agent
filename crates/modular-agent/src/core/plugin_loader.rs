//! Dylib plugin loader.
//!
//! Сканирует папку `~/.agent/plugins/` (или указанную) на `*.so`/`*.dylib`/
//! `*.dll`, загружает через abi_stable, даёт каждому плагину callback для
//! регистрации модулей в Registry.
//!
//! При ошибке загрузки (несовместимый ABI, отсутствие export'а, panic в
//! register_modules) плагин пропускается с warning в stderr. Ядро продолжает
//! работать с оставшимися плагинами и builtin-модулями.

use std::path::{Path, PathBuf};

use anyhow::Result;
use agent_contracts::{
    abi_stable::{
        library::{LibraryError, RootModule},
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    contracts::RendererObject,
    plugin::{PluginRegistry, PluginRegistry_TO, PluginRegisterError, PluginRoot_Ref},
};

use crate::core::BuiltinModuleCatalog;

/// Адаптер, через который плагин регистрирует свои модули в ядре.
///
/// Плагин видит этот объект как sabi_trait `PluginRegistry`. Ядро держит
/// ссылку на `BuiltinModuleCatalog` и переводит вызовы плагина в
/// обычные registrations.
struct PluginRegistryAdapter<'a> {
    catalog: &'a mut BuiltinModuleCatalog,
}

impl<'a> PluginRegistry for PluginRegistryAdapter<'a> {
    fn register_renderer(
        &mut self,
        module_id: RString,
        renderer: RendererObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_renderer(&id, renderer) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }
}

/// Итог попытки загрузить один плагин.
#[derive(Debug)]
pub struct PluginLoadReport {
    pub path: PathBuf,
    pub result: Result<PluginInfo>,
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// Сканирует папку плагинов и загружает каждый найденный dylib.
///
/// Возвращает отчёт по каждому найденному файлу. Успешно загруженные
/// плагины уже зарегистрированы в `catalog`; неуспешные не повлияли на
/// состояние catalog.
pub fn load_plugins_from_dir(
    plugins_dir: &Path,
    catalog: &mut BuiltinModuleCatalog,
) -> Vec<PluginLoadReport> {
    let mut reports = Vec::new();

    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Нет папки плагинов — это нормально, плагинов просто нет.
            return reports;
        }
        Err(error) => {
            eprintln!(
                "warning: could not read plugins directory {}: {}",
                plugins_dir.display(),
                error
            );
            return reports;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_dylib_file(&path) {
            continue;
        }
        let report = load_one_plugin(&path, catalog);
        if let Err(ref error) = report.result {
            eprintln!(
                "warning: failed to load plugin {}: {}",
                path.display(),
                error
            );
        }
        reports.push(report);
    }

    reports
}

fn is_dylib_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(ext, "so" | "dylib" | "dll")
}

fn load_one_plugin(path: &Path, catalog: &mut BuiltinModuleCatalog) -> PluginLoadReport {
    let result = load_one_plugin_inner(path, catalog);
    PluginLoadReport {
        path: path.to_path_buf(),
        result,
    }
}

fn load_one_plugin_inner(
    path: &Path,
    catalog: &mut BuiltinModuleCatalog,
) -> Result<PluginInfo> {
    let root: PluginRoot_Ref = PluginRoot_Ref::load_from_file(path).map_err(map_library_error)?;

    let name = root.name().as_str().to_string();
    let description = root.description().as_str().to_string();

    let register_fn = root.register_modules();
    let mut adapter = PluginRegistryAdapter { catalog };
    let mut registry_to: PluginRegistry_TO<_> =
        PluginRegistry_TO::from_ptr(&mut adapter, TD_Opaque);
    match register_fn(&mut registry_to) {
        RResult::ROk(()) => Ok(PluginInfo {
            name,
            description,
            path: path.to_path_buf(),
        }),
        RResult::RErr(err) => Err(anyhow::anyhow!(
            "plugin '{}' register_modules failed: {}",
            name,
            err.message
        )),
    }
}

fn map_library_error(err: LibraryError) -> anyhow::Error {
    anyhow::anyhow!("abi_stable load error: {err}")
}

/// Возвращает стандартный путь к папке плагинов.
///
/// Порядок разрешения:
/// 1. `$AGENT_PLUGINS_DIR` если задан.
/// 2. `~/.agent/plugins` иначе.
pub fn default_plugins_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("AGENT_PLUGINS_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".agent").join("plugins"))
}
