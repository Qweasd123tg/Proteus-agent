//! ZIP payloads belong to the device, outside the replaceable portable folder.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use uuid::Uuid;

use super::manifest::{Manifest, relative_path, resource_path};

pub(super) const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 4096;

#[derive(Debug, Serialize)]
pub(crate) struct InstalledPackage {
    pub id: String,
    pub key: String,
    pub url: String,
}

pub(super) struct PackageStore {
    root: PathBuf,
}

impl PackageStore {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(super) fn install(
        &self,
        bytes: &[u8],
        excluded_ids: &[String],
    ) -> Result<InstalledPackage> {
        ensure!(bytes.len() <= MAX_ARCHIVE_BYTES, "ZIP превышает 64 МиБ");
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).context("Не удалось прочитать ZIP")?;
        ensure!(
            archive.len() <= MAX_FILES,
            "В ZIP больше {MAX_FILES} записей"
        );
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let path = relative_path(entry.name())?;
            ensure!(
                entry
                    .unix_mode()
                    .is_none_or(|mode| mode & 0o170000 != 0o120000),
                "Ссылки в ZIP не поддерживаются"
            );
            if entry.is_dir() {
                continue;
            }
            total = total
                .checked_add(entry.size())
                .context("Неверный размер ZIP")?;
            ensure!(
                total <= MAX_EXPANDED_BYTES,
                "Распакованный ZIP превышает 128 МиБ"
            );
            let mut content = Vec::new();
            let size = entry.size();
            entry.by_ref().take(size + 1).read_to_end(&mut content)?;
            ensure!(content.len() as u64 == size, "Неверный размер файла в ZIP");
            ensure!(
                files.insert(path, content).is_none(),
                "Повторный путь файла в ZIP"
            );
        }
        let manifest_bytes = files
            .get(Path::new("extension.json"))
            .context("В корне ZIP нужен extension.json")?;
        let manifest: Manifest =
            serde_json::from_slice(manifest_bytes).context("Неверный манифест расширения")?;
        manifest.validate(&files.keys().cloned().collect::<BTreeSet<_>>())?;
        ensure!(
            !excluded_ids.contains(&manifest.id),
            "Расширение {} уже добавлено или имя занято встроенной страницей",
            manifest.id
        );
        std::fs::create_dir_all(&self.root)?;
        let key = Uuid::new_v4().to_string();
        let temporary = self.root.join(format!(".install-{key}"));
        let destination = self.root.join(&key);
        let result = (|| -> Result<()> {
            std::fs::create_dir(&temporary)?;
            for (path, content) in &files {
                let file = temporary.join(path);
                std::fs::create_dir_all(file.parent().unwrap())?;
                std::fs::write(file, content)?;
            }
            std::fs::rename(&temporary, &destination)?;
            Ok(())
        })();
        if result.is_err() && temporary.exists() {
            let _ = std::fs::remove_dir_all(&temporary);
        }
        result?;
        let origin = if cfg!(any(target_os = "windows", target_os = "android")) {
            "http://proteus-extension.localhost"
        } else {
            "proteus-extension://localhost"
        };
        Ok(InstalledPackage {
            id: manifest.id,
            url: format!("{origin}/{key}/extension.json"),
            key,
        })
    }

    fn package(&self, key: &str) -> Result<PathBuf> {
        ensure!(
            Uuid::parse_str(key)?.to_string() == key,
            "Неверный ключ пакета"
        );
        Ok(self.root.join(key))
    }

    pub(super) fn remove(&self, key: &str) -> Result<()> {
        let path = self.package(key)?;
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    pub(super) fn read(&self, key: &str, resource: &str) -> Result<(Vec<u8>, String)> {
        let path = resource_path(resource)?;
        let file = self.package(key)?.join(&path);
        let canonical = file.canonicalize()?;
        ensure!(
            canonical.starts_with(self.package(key)?.canonicalize()?),
            "Ресурс выходит за пределы пакета"
        );
        let mime = if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("js" | "mjs")
        ) {
            "text/javascript".to_owned()
        } else {
            mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string()
        };
        Ok((std::fs::read(canonical)?, mime))
    }
}
