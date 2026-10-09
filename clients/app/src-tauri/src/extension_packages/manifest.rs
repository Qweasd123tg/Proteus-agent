//! Installation validation for the client-owned UI API v4, not agent contracts.
use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    path::{Component, PathBuf},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    api_version: u8,
    pub id: String,
    name: String,
    description: String,
    #[serde(default, deserialize_with = "present")]
    icon: Option<Icon>,
    #[serde(default, deserialize_with = "present")]
    preview: Option<Preview>,
    views: Vec<View>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Icon {
    src: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {
    entry: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct View {
    surfaces: Vec<String>,
    entry: String,
    requires: Vec<String>,
    layout: String,
    isolation: String,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub(super) fn relative_path(value: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty() && !value.contains(['\\', '\0', ':']),
        "Нужен относительный путь внутри ZIP: {value:?}"
    );
    let mut path = PathBuf::new();
    for component in std::path::Path::new(value).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => (),
            _ => bail!("Путь выходит за пределы пакета: {value:?}"),
        }
    }
    ensure!(!path.as_os_str().is_empty(), "Пустой путь файла пакета");
    Ok(path)
}

pub(super) fn resource_path(value: &str) -> Result<PathBuf> {
    let value = value.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(value).decode_utf8()?;
    relative_path(&decoded)
}

impl Manifest {
    pub(super) fn validate(&self, files: &BTreeSet<PathBuf>) -> Result<()> {
        ensure!(
            self.api_version == 4,
            "Неподдерживаемая версия UI API: {}",
            self.api_version
        );
        ensure!(
            self.id.split(['.', '-']).all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())),
            "Некорректный id расширения"
        );
        ensure!(
            !self.name.trim().is_empty(),
            "Не указано название расширения"
        );
        // Descriptions may be empty, but their type is part of the strict DTO.
        let _ = &self.description;
        let check_file = |value: &str| -> Result<()> {
            ensure!(
                value == value.trim() && !value.contains('#'),
                "Некорректный URL ресурса: {value:?}"
            );
            let path = resource_path(value)?;
            ensure!(files.contains(&path), "В ZIP отсутствует ресурс {value:?}");
            Ok(())
        };
        if let Some(icon) = &self.icon {
            check_file(&icon.src)?;
        }
        let check_entry = |value: &str| -> Result<()> {
            check_file(value)?;
            ensure!(
                matches!(
                    resource_path(value)?.extension().and_then(|s| s.to_str()),
                    Some("js" | "mjs")
                ),
                "Entry должен быть модулем JavaScript (.js или .mjs)"
            );
            Ok(())
        };
        if let Some(preview) = &self.preview {
            check_entry(&preview.entry)?;
        }
        ensure!(
            !self.views.is_empty(),
            "Нужен список представлений расширения"
        );
        let mut declared = BTreeSet::new();
        for view in &self.views {
            ensure!(
                !view.surfaces.is_empty(),
                "Нужен список поверхностей представления"
            );
            for surface in &view.surfaces {
                ensure!(
                    [
                        "compact",
                        "workspace",
                        "settings",
                        "composer-model",
                        "composer-access"
                    ]
                    .contains(&surface.as_str())
                        && declared.insert(surface),
                    "Неизвестная или повторная поверхность: {surface}"
                );
            }
            ensure!(
                view.surfaces.len() == 1
                    || view
                        .surfaces
                        .iter()
                        .all(|s| ["compact", "workspace"].contains(&s.as_str())),
                "Общий экземпляр допустим только для compact и workspace"
            );
            let mut services = BTreeSet::new();
            ensure!(
                view.requires
                    .iter()
                    .all(|name| !name.trim().is_empty() && services.insert(name)),
                "Некорректные интерфейсы представления"
            );
            ensure!(
                ["scroll", "fill", "form", "editor"].contains(&view.layout.as_str()),
                "Неизвестный layout представления"
            );
            ensure!(
                ["shadow", "light"].contains(&view.isolation.as_str()),
                "Неизвестная изоляция представления"
            );
            check_entry(&view.entry)?;
        }
        Ok(())
    }
}
