//! Read-only configuration discovery, independent of behavior slot dispatch.
use crate::domain::ModuleConfigSchema;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct ConfigSchemas {
    pub schemas: BTreeMap<(String, String), ModuleConfigSchema>,
    pub errors: Vec<String>,
}

impl super::RuntimeRegistry {
    pub(crate) async fn config_schemas(&self) -> ConfigSchemas {
        let exports = self.config_exports.clone();
        let cwd = self.cwd.clone();
        match tokio::task::spawn_blocking(move || {
            let mut result = ConfigSchemas::default();
            let mut seen = BTreeSet::new();
            for export in exports {
                if !seen.insert(export.component_id().to_owned()) {
                    continue;
                }
                match export.connect(&cwd).and_then(|broker| broker.manifest()) {
                    Ok(manifest) => {
                        for export in manifest.exports {
                            if let Some(schema) = export.config_schema {
                                result
                                    .schemas
                                    .insert((export.slot, export.module_id), schema);
                            }
                        }
                    }
                    Err(error) => result.errors.push(format!(
                        "Не удалось прочитать описание настроек компонента {}: {error:#}",
                        export.component_id()
                    )),
                }
            }
            result
        })
        .await
        {
            Ok(result) => result,
            Err(error) => ConfigSchemas {
                schemas: BTreeMap::new(),
                errors: vec![format!("Не удалось прочитать описание настроек: {error}")],
            },
        }
    }
}
