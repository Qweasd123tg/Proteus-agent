//! Dylib plugin loader.
//!
//! Сканирует папку `~/.agent/plugins/` на `*.so`/`*.dylib`/`*.dll`, загружает
//! каждый через `libloading`, находит экспорт через abi_stable
//! `ROOT_MODULE_LOADER_NAME`, даёт плагину callback для регистрации модулей.
//!
//! ## Почему libloading напрямую, а не `RootModule::load_from_file`
//!
//! Высокоуровневый `load_from_file` кеширует root module **по типу** в
//! `'static` slot'е. При загрузке второго плагина того же типа (`PluginRoot`)
//! он возвращает первый, что ломает multi-plugin сценарий. Поэтому мы
//! используем `libloading` + `AbiHeaderRef::upgrade` + `init_root_module`
//! напрямую, что даёт независимый root module на каждый dylib.
//!
//! При ошибке загрузки (несовместимый ABI, отсутствие export'а, panic в
//! register_modules) плагин пропускается с warning в stderr. Ядро продолжает
//! работать с оставшимися плагинами и builtin-модулями.

use std::path::{Path, PathBuf};

use agent_contracts::{
    abi_stable::{
        library::{LibHeader, RawLibrary, lib_header_from_raw_library},
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    contracts::RendererObject,
    plugin::{
        CompactorObject, ContextBuilderObject, ContextProviderObject, MemoryPolicyObject,
        MemoryStoreObject, PatchApplierObject, PluginRegisterError, PluginRegistry,
        PluginRegistry_TO, PluginRoot_Ref, PluginToolObject, PolicyObject, SearchBackendObject,
        ToolExposureObject, WorkflowObject,
    },
};
use anyhow::Result;

use crate::core::BuiltinModuleCatalog;

/// Адаптер, через который плагин регистрирует свои модули в ядре.
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

    fn register_tool(&mut self, tool: PluginToolObject) -> RResult<(), PluginRegisterError> {
        match self.catalog.register_plugin_tool(tool) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_approval_policy(
        &mut self,
        module_id: RString,
        policy: PolicyObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_policy(&id, policy) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_patch_applier(
        &mut self,
        module_id: RString,
        applier: PatchApplierObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_patch(&id, applier) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_search_backend(
        &mut self,
        module_id: RString,
        backend: SearchBackendObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_search_backend(&id, backend) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_memory_store(
        &mut self,
        module_id: RString,
        store: MemoryStoreObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_memory_store(&id, store) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_context_provider(
        &mut self,
        provider_id: RString,
        provider: ContextProviderObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = provider_id.into_string();
        match self.catalog.register_plugin_context_provider(&id, provider) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_context_builder(
        &mut self,
        module_id: RString,
        builder: ContextBuilderObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_context_builder(&id, builder) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_memory_policy(
        &mut self,
        module_id: RString,
        policy: MemoryPolicyObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_memory_policy(&id, policy) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_compactor(
        &mut self,
        module_id: RString,
        compactor: CompactorObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_compactor(&id, compactor) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_tool_exposure(
        &mut self,
        module_id: RString,
        exposure: ToolExposureObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_tool_exposure(&id, exposure) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }

    fn register_workflow(
        &mut self,
        module_id: RString,
        workflow: WorkflowObject,
    ) -> RResult<(), PluginRegisterError> {
        let id = module_id.into_string();
        match self.catalog.register_plugin_workflow(&id, workflow) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginRegisterError::new(error.to_string())),
        }
    }
}

#[derive(Debug)]
pub struct PluginLoadReport {
    pub path: PathBuf,
    /// Manifest из plugin.toml, если он был прочитан до попытки загрузки
    /// dylib. Остаётся доступен даже если последующая загрузка провалилась —
    /// `modules list` может показать метаданные плагина вместе с причиной
    /// ошибки.
    pub manifest: Option<PluginManifest>,
    pub result: Result<PluginInfo>,
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    /// Если рядом с .so был найден `plugin.toml`, его содержимое попадает сюда.
    /// Позволяет получить metadata плагина (version, author, tags) без
    /// зависимости от значений, которые плагин самообъявляет внутри PluginRoot.
    pub manifest: Option<PluginManifest>,
}

/// Метаданные плагина из `plugin.toml` рядом с .so.
///
/// Manifest необязателен. Если есть — читается до загрузки .so (т.е. даже
/// несовместимый по ABI плагин виден в `modules list` с пометкой, что он
/// не загрузился).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginManifest {
    /// Человекочитаемое имя плагина.
    pub name: String,

    /// Версия плагина (semver-like строка).
    pub version: String,

    /// Короткое описание.
    #[serde(default)]
    pub description: Option<String>,

    /// Автор/поддержка.
    #[serde(default)]
    pub author: Option<String>,

    /// Список тегов/категорий.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Имя .so/.dylib/.dll файла рядом с manifest'ом. Если не указано —
    /// loader ищет любой .so в той же папке.
    #[serde(default)]
    pub library: Option<String>,

    /// Требуемая версия agent-contracts для информационных целей. Реальная
    /// проверка совместимости — через abi_stable layout check при load.
    #[serde(default)]
    pub requires_agent_contracts: Option<String>,
}

pub fn load_plugins_from_dir(
    plugins_dir: &Path,
    catalog: &mut BuiltinModuleCatalog,
) -> Vec<PluginLoadReport> {
    // Escape hatch для тестов и для запуска без плагинов:
    // `AGENT_PLUGINS_DISABLE=1` полностью отключает сканирование.
    if std::env::var_os("AGENT_PLUGINS_DISABLE").is_some() {
        return Vec::new();
    }
    scan_plugins_dir(plugins_dir, catalog)
}

/// Внутренний вариант `load_plugins_from_dir`, не смотрящий на env.
/// Полезен в unit-тестах, которые не должны мутировать глобальные переменные.
fn scan_plugins_dir(
    plugins_dir: &Path,
    catalog: &mut BuiltinModuleCatalog,
) -> Vec<PluginLoadReport> {
    let mut reports = Vec::new();

    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return reports,
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

        // Вариант 1: папка `plugin-name/` с plugin.toml внутри.
        if path.is_dir() {
            let manifest_path = path.join("plugin.toml");
            if manifest_path.exists() {
                let report = load_from_manifest_dir(&path, &manifest_path, catalog);
                if let Err(ref error) = report.result {
                    eprintln!(
                        "warning: failed to load plugin {}: {}",
                        path.display(),
                        error
                    );
                }
                reports.push(report);
            }
            // Папки без plugin.toml игнорируем — они могут быть чем-то
            // другим (например, не-плагины). Не скандалим.
            continue;
        }

        // Вариант 2: просто .so/.dylib/.dll в корне папки плагинов.
        if !is_dylib_file(&path) {
            continue;
        }
        let report = load_one_plugin(&path, None, catalog);
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

fn load_from_manifest_dir(
    plugin_dir: &Path,
    manifest_path: &Path,
    catalog: &mut BuiltinModuleCatalog,
) -> PluginLoadReport {
    let manifest = match read_manifest(manifest_path) {
        Ok(m) => m,
        Err(error) => {
            return PluginLoadReport {
                path: manifest_path.to_path_buf(),
                manifest: None,
                result: Err(error),
            };
        }
    };

    // Путь к .so: либо явно указан в manifest.library, либо ищем единственный
    // dylib в папке.
    let lib_path = match manifest.library.as_deref() {
        Some(name) => plugin_dir.join(name),
        None => match find_single_dylib(plugin_dir) {
            Ok(path) => path,
            Err(error) => {
                return PluginLoadReport {
                    path: plugin_dir.to_path_buf(),
                    manifest: Some(manifest),
                    result: Err(error),
                };
            }
        },
    };

    load_one_plugin(&lib_path, Some(manifest), catalog)
}

fn read_manifest(path: &Path) -> Result<PluginManifest> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    toml::from_str(&content).map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))
}

fn find_single_dylib(dir: &Path) -> Result<PathBuf> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", dir.display()))?;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if is_dylib_file(&path) {
            candidates.push(path);
        }
    }
    match candidates.len() {
        0 => anyhow::bail!("no dylib found in {}", dir.display()),
        1 => Ok(candidates.into_iter().next().unwrap()),
        _ => anyhow::bail!(
            "multiple dylibs in {}; specify `library` in plugin.toml",
            dir.display()
        ),
    }
}

fn is_dylib_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(ext, "so" | "dylib" | "dll")
}

fn load_one_plugin(
    path: &Path,
    manifest: Option<PluginManifest>,
    catalog: &mut BuiltinModuleCatalog,
) -> PluginLoadReport {
    // Сохраняем manifest для отчёта даже если загрузка .so упадёт.
    let report_manifest = manifest.clone();
    let result = load_one_plugin_inner(path, manifest, catalog);
    PluginLoadReport {
        path: path.to_path_buf(),
        manifest: report_manifest,
        result,
    }
}

fn load_one_plugin_inner(
    path: &Path,
    manifest: Option<PluginManifest>,
    catalog: &mut BuiltinModuleCatalog,
) -> Result<PluginInfo> {
    // Загружаем raw library через abi_stable (он сам leak'нёт чтобы символы
    // оставались валидными — это требуется потому что мы потом держим trait
    // объекты из этого dylib на всё время жизни процесса).
    let raw_lib =
        RawLibrary::load_at(path).map_err(|err| anyhow::anyhow!("failed to load dylib: {err}"))?;

    // Получаем LibHeader — abi_stable проверяет что версия abi_stable в
    // плагине совместима с нашей.
    let lib_header: &LibHeader = unsafe {
        lib_header_from_raw_library(&raw_lib)
            .map_err(|err| anyhow::anyhow!("failed to read abi_stable header: {err}"))?
    };

    // Проверяем layout PluginRoot в плагине против нашего текущего.
    // Если плагин был собран против более старой/новой несовместимой версии
    // agent-contracts, вот здесь это вылезет.
    lib_header
        .ensure_layout::<PluginRoot_Ref>()
        .map_err(|err| anyhow::anyhow!("ABI layout mismatch: {err}"))?;

    // init_root_module возвращает свежий PluginRoot_Ref каждый раз (он не
    // привязан к type-keyed cache, который портил RootModule::load_from_file).
    let root: PluginRoot_Ref = lib_header
        .init_root_module::<PluginRoot_Ref>()
        .map_err(|err| anyhow::anyhow!("failed to init root module: {err}"))?;

    // Приоритет: manifest переопределяет значения из PluginRoot. Manifest
    // читается до загрузки .so, поэтому его имя и описание — authoritative
    // для listing'а. Если manifest'а нет или поле пустое — fallback на
    // самообъявленные плагином значения.
    let root_name = root.name().as_str().to_string();
    let root_description = root.description().as_str().to_string();
    let name = manifest
        .as_ref()
        .map(|m| m.name.clone())
        .unwrap_or(root_name.clone());
    let description = manifest
        .as_ref()
        .and_then(|m| m.description.clone())
        .unwrap_or(root_description);

    let register_fn = root.register_modules();
    let checkpoint = catalog.checkpoint();
    let mut adapter = PluginRegistryAdapter { catalog };
    let mut registry_to: PluginRegistry_TO<_> =
        PluginRegistry_TO::from_ptr(&mut adapter, TD_Opaque);
    match register_fn(&mut registry_to) {
        RResult::ROk(()) => {
            drop(registry_to);
            drop(adapter);
            // Важно: leak'аем RawLibrary только после успешной регистрации —
            // иначе при drop символы плагина станут dangling, а trait objects
            // из этого dylib живут в catalog всё время процесса.
            std::mem::forget(raw_lib);
            Ok(PluginInfo {
                name,
                description,
                path: path.to_path_buf(),
                manifest,
            })
        }
        RResult::RErr(err) => {
            drop(registry_to);
            drop(adapter);
            catalog.rollback_to(checkpoint);
            Err(anyhow::anyhow!(
                "plugin '{}' register_modules failed: {}",
                root_name,
                err.message
            ))
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_manifest_parses_full_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        std::fs::write(
            &path,
            r#"
name = "sample"
version = "1.2.3"
description = "a sample plugin"
author = "me"
tags = ["demo", "test"]
library = "libsample.so"
requires_agent_contracts = "^0.1"
"#,
        )
        .unwrap();

        let manifest = read_manifest(&path).unwrap();
        assert_eq!(manifest.name, "sample");
        assert_eq!(manifest.version, "1.2.3");
        assert_eq!(manifest.description.as_deref(), Some("a sample plugin"));
        assert_eq!(manifest.tags, vec!["demo", "test"]);
        assert_eq!(manifest.library.as_deref(), Some("libsample.so"));
    }

    #[test]
    fn read_manifest_accepts_minimal_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        std::fs::write(&path, "name = \"x\"\nversion = \"0.0.1\"\n").unwrap();
        let manifest = read_manifest(&path).unwrap();
        assert_eq!(manifest.name, "x");
        assert!(manifest.description.is_none());
        assert!(manifest.tags.is_empty());
        assert!(manifest.library.is_none());
    }

    #[test]
    fn read_manifest_errors_on_broken_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        std::fs::write(&path, "name = not-valid-toml").unwrap();
        let error = read_manifest(&path).unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }

    #[test]
    fn find_single_dylib_errors_when_none() {
        let dir = tempdir().unwrap();
        let error = find_single_dylib(dir.path()).unwrap_err();
        assert!(error.to_string().contains("no dylib found"));
    }

    #[test]
    fn find_single_dylib_errors_when_multiple() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.so"), b"").unwrap();
        std::fs::write(dir.path().join("b.so"), b"").unwrap();
        let error = find_single_dylib(dir.path()).unwrap_err();
        assert!(error.to_string().contains("multiple dylibs"));
    }

    #[test]
    fn find_single_dylib_returns_the_only_match() {
        let dir = tempdir().unwrap();
        let expected = dir.path().join("only.so");
        std::fs::write(&expected, b"").unwrap();
        std::fs::write(dir.path().join("ignore.txt"), b"").unwrap();
        let path = find_single_dylib(dir.path()).unwrap();
        assert_eq!(path, expected);
    }

    #[test]
    fn scan_plugins_dir_surfaces_broken_manifest() {
        let plugins_dir = tempdir().unwrap();
        let sub = plugins_dir.path().join("broken");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("plugin.toml"), "not = valid = toml").unwrap();

        let mut catalog = BuiltinModuleCatalog::new();
        let reports = scan_plugins_dir(plugins_dir.path(), &mut catalog);

        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert!(
            report.manifest.is_none(),
            "broken manifest should not be kept"
        );
        let error = report.result.as_ref().unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }

    #[test]
    fn scan_plugins_dir_keeps_manifest_when_dylib_missing() {
        let plugins_dir = tempdir().unwrap();
        let sub = plugins_dir.path().join("ghost");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("plugin.toml"),
            "name = \"ghost\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let mut catalog = BuiltinModuleCatalog::new();
        let reports = scan_plugins_dir(plugins_dir.path(), &mut catalog);

        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.manifest.as_ref().unwrap().name, "ghost");
        let error = report.result.as_ref().unwrap_err();
        assert!(error.to_string().contains("no dylib found"));
    }

    #[test]
    fn scan_plugins_dir_ignores_folders_without_manifest() {
        let plugins_dir = tempdir().unwrap();
        let sub = plugins_dir.path().join("no-manifest");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("readme.txt"), "hi").unwrap();

        let mut catalog = BuiltinModuleCatalog::new();
        let reports = scan_plugins_dir(plugins_dir.path(), &mut catalog);
        assert!(reports.is_empty());
    }
}
