use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

const ROOT_WORKSPACE_DIR: &str = "%2F";

pub fn encode_workspace_path(path: &Path) -> Result<String> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !path.is_absolute() {
        bail!("workspace path must be absolute: {}", path.display());
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| {
                    anyhow!(
                        "workspace path contains a non-UTF-8 component: {}",
                        path.display()
                    )
                })?;
                parts.push(encode_workspace_component(part));
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                bail!("unsupported workspace path: {}", path.display());
            }
        }
    }

    if parts.is_empty() {
        Ok(ROOT_WORKSPACE_DIR.to_owned())
    } else {
        Ok(parts.join("|"))
    }
}

pub fn decode_workspace_path(encoded: &str) -> Result<PathBuf> {
    let path = if encoded == ROOT_WORKSPACE_DIR {
        PathBuf::from("/")
    } else {
        if encoded.is_empty() {
            bail!("encoded workspace directory name must not be empty");
        }

        let mut path = PathBuf::from("/");
        for encoded_part in encoded.split('|') {
            let part = decode_workspace_component(encoded_part)?;
            let mut components = Path::new(&part).components();
            if !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
            {
                bail!("invalid encoded workspace path component {encoded_part:?}");
            }
            path.push(part);
        }
        path
    };

    let canonical = std::fs::canonicalize(&path)
        .with_context(|| format!("encoded workspace path does not exist: {}", path.display()))?;
    let canonical_encoded = encode_workspace_path(&canonical)?;
    if canonical_encoded != encoded {
        bail!(
            "workspace directory name {encoded:?} is not canonical for {} (expected {canonical_encoded:?})",
            canonical.display()
        );
    }
    Ok(canonical)
}

pub(super) fn workspace_path_from_session_dir(session_dir: &Path) -> Result<PathBuf> {
    let workspace_dir = session_dir.parent().ok_or_else(|| {
        anyhow!(
            "session directory has no workspace parent: {}",
            session_dir.display()
        )
    })?;
    let encoded = workspace_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "session workspace directory must have a UTF-8 name: {}",
                workspace_dir.display()
            )
        })?;
    decode_workspace_path(encoded)
}

fn encode_workspace_component(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            let mut bytes = [0_u8; 4];
            for byte in ch.encode_utf8(&mut bytes).as_bytes() {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    out
}

fn decode_workspace_component(encoded: &str) -> Result<String> {
    if encoded.is_empty() {
        bail!("encoded workspace path contains an empty component");
    }

    let input = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] != b'%' {
            decoded.push(input[index]);
            index += 1;
            continue;
        }
        if index + 2 >= input.len() {
            bail!("invalid percent escape in workspace component {encoded:?}");
        }
        let high = decode_hex(input[index + 1])
            .ok_or_else(|| anyhow!("invalid percent escape in workspace component {encoded:?}"))?;
        let low = decode_hex(input[index + 2])
            .ok_or_else(|| anyhow!("invalid percent escape in workspace component {encoded:?}"))?;
        decoded.push((high << 4) | low);
        index += 3;
    }

    let decoded = String::from_utf8(decoded)
        .with_context(|| format!("workspace component is not valid UTF-8: {encoded:?}"))?;
    if encode_workspace_component(&decoded) != encoded {
        bail!("workspace component is not canonically encoded: {encoded:?}");
    }
    Ok(decoded)
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_directory_name_round_trips_special_characters() {
        let root = tempfile::tempdir().expect("workspace root");
        let workspace = root.path().join("Проекты").join("моя игра|100%");
        std::fs::create_dir_all(&workspace).expect("workspace");

        let encoded = encode_workspace_path(&workspace).expect("encoded workspace");
        let decoded = decode_workspace_path(&encoded).expect("decoded workspace");

        assert!(encoded.ends_with("Проекты|моя%20игра%7C100%25"));
        assert_eq!(decoded, workspace);
    }

    #[test]
    fn decoder_rejects_noncanonical_and_missing_workspace_names() {
        let root = tempfile::tempdir().expect("workspace root");
        let encoded = encode_workspace_path(root.path()).expect("encoded workspace");
        let noncanonical = encoded.replace("tmp", "%74mp");

        let noncanonical_error =
            decode_workspace_path(&noncanonical).expect_err("noncanonical encoding");
        let missing_error = decode_workspace_path("definitely|missing|proteus|workspace")
            .expect_err("missing workspace");

        assert!(
            noncanonical_error
                .to_string()
                .contains("not canonically encoded")
        );
        assert!(missing_error.to_string().contains("does not exist"));
    }
}
