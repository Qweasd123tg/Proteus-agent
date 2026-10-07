use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use proteus_contracts::domain::SkillRuntimeSettings;
use serde::Deserialize;

const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;
const MAX_SKILLS: usize = 128;
const MAX_DESCRIPTION_CHARACTERS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SkillSource {
    User,
    Project,
    Plugin,
}

impl SkillSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Plugin => "plugin",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SkillDocument {
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) body: String,
    pub(crate) path: PathBuf,
    pub(crate) source: SkillSource,
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(rename = "license")]
    _license: Option<String>,
    compatibility: Option<String>,
    #[serde(rename = "metadata")]
    _metadata: Option<BTreeMap<String, String>>,
    #[serde(rename = "allowed-tools")]
    _allowed_tools: Option<String>,
}

pub(crate) fn discover_skills(
    cwd: &Path,
    settings: &SkillRuntimeSettings,
) -> Result<(Vec<SkillDocument>, Vec<String>), String> {
    let project_root = workspace_root(cwd).join(".proteus/skills");
    let user_root = user_skills_root();
    let mut skills = discover_from_roots(user_root.as_deref(), &project_root)?;
    let mut warnings = Vec::new();
    for package in &settings.packages {
        let root = package.root.join("skills");
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                warnings.push(format!("{}: {error}", root.display()));
                continue;
            }
        };
        for entry in entries {
            let result = (|| {
                let directory = entry.map_err(|error| error.to_string())?.path();
                let path = directory.join("SKILL.md");
                if !directory.is_dir() || !path.is_file() {
                    return Ok(None);
                }
                let path = path.canonicalize().map_err(|error| error.to_string())?;
                if !path.starts_with(&package.root) {
                    return Err(format!(
                        "plugin skill escapes package root: {}",
                        path.display()
                    ));
                }
                let name = directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| "skill directory name is not UTF-8".to_string())?;
                let mut skill = parse_skill_file(&path, name, SkillSource::Plugin)?;
                skill.id = format!("{}:{}", package.id, skill.name);
                skill.enabled = package.enabled;
                Ok(Some(skill))
            })();
            match result {
                Ok(Some(skill)) => skills.push(skill),
                Ok(None) => {}
                Err(error) => warnings.push(error),
            }
        }
    }
    if skills.len() > MAX_SKILLS {
        return Err(format!("skill discovery exceeds {MAX_SKILLS} skills"));
    }
    for skill in &mut skills {
        skill.enabled &= !settings.disabled.contains(&skill.id);
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((skills, warnings))
}

fn user_skills_root() -> Option<PathBuf> {
    env::var_os("PROTEUS_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".proteus")))
        .map(|root| root.join("skills"))
}

fn workspace_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

pub(crate) fn discover_from_roots(
    user_root: Option<&Path>,
    project_root: &Path,
) -> Result<Vec<SkillDocument>, String> {
    let mut skills = BTreeMap::new();
    if let Some(root) = user_root {
        insert_root(&mut skills, root, SkillSource::User, false)?;
    }
    insert_root(&mut skills, project_root, SkillSource::Project, true)?;
    if skills.len() > MAX_SKILLS {
        return Err(format!(
            "skill discovery found {} skills; maximum is {MAX_SKILLS}",
            skills.len()
        ));
    }
    Ok(skills.into_values().collect())
}

fn insert_root(
    skills: &mut BTreeMap<String, SkillDocument>,
    root: &Path,
    source: SkillSource,
    replace_existing: bool,
) -> Result<(), String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to read {} skills root {}: {error}",
                source.as_str(),
                root.display()
            ));
        }
    };
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve skills root {}: {error}", root.display()))?;
    let mut candidates = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to enumerate skills root {}: {error}",
                root.display()
            )
        })?;
    candidates.sort();

    let mut root_names = BTreeMap::<String, PathBuf>::new();
    for directory in candidates {
        if !directory.is_dir() {
            continue;
        }
        let path = directory.join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let canonical_path = path
            .canonicalize()
            .map_err(|error| format!("failed to resolve {}: {error}", path.display()))?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(format!(
                "skill file escapes its {} root: {}",
                source.as_str(),
                path.display()
            ));
        }
        let directory_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("skill directory name is not UTF-8: {}", directory.display()))?;
        let document = parse_skill_file(&canonical_path, directory_name, source)?;
        if let Some(previous) = root_names.insert(document.name.clone(), canonical_path.clone()) {
            return Err(format!(
                "duplicate skill name '{}' in {} and {}",
                document.name,
                previous.display(),
                canonical_path.display()
            ));
        }
        if replace_existing || !skills.contains_key(&document.name) {
            skills.insert(document.name.clone(), document);
        }
    }
    Ok(())
}

fn parse_skill_file(
    path: &Path,
    directory_name: &str,
    source: SkillSource,
) -> Result<SkillDocument, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_SKILL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "skill file exceeds {MAX_SKILL_FILE_BYTES} bytes: {}",
            path.display()
        ));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| format!("skill file is not UTF-8: {}", path.display()))?;
    let (yaml, body) = split_frontmatter(&content)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml)
        .map_err(|error| format!("invalid YAML frontmatter in {}: {error}", path.display()))?;
    validate_name(&frontmatter.name)?;
    if frontmatter
        .compatibility
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.chars().count() > 500)
    {
        return Err("invalid skill compatibility field".into());
    }
    if frontmatter.name != directory_name {
        return Err(format!(
            "skill name '{}' must match directory '{}': {}",
            frontmatter.name,
            directory_name,
            path.display()
        ));
    }
    let description = frontmatter.description.trim().to_owned();
    if description.is_empty() {
        return Err(format!(
            "skill description must not be empty: {}",
            path.display()
        ));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARACTERS {
        return Err(format!(
            "skill description exceeds {MAX_DESCRIPTION_CHARACTERS} characters: {}",
            path.display()
        ));
    }
    let body = body.trim().to_owned();
    Ok(SkillDocument {
        id: frontmatter.name.clone(),
        enabled: true,
        name: frontmatter.name,
        description,
        body,
        path: path.to_path_buf(),
        source,
    })
}

fn split_frontmatter(content: &str) -> Result<(&str, &str), &'static str> {
    let opening_len = if content.starts_with("---\n") {
        4
    } else if content.starts_with("---\r\n") {
        5
    } else {
        return Err("missing opening YAML frontmatter marker");
    };
    let remainder = &content[opening_len..];
    let mut offset = opening_len;
    for line in remainder.split_inclusive('\n') {
        let line_without_ending = line.trim_end_matches(['\r', '\n']);
        if line_without_ending == "---" {
            let yaml = &content[opening_len..offset];
            let body = &content[offset + line.len()..];
            return Ok((yaml, body));
        }
        offset += line.len();
    }
    if remainder.ends_with("---") {
        return Ok((&content[opening_len..content.len() - 3], ""));
    }
    Err("missing closing YAML frontmatter marker")
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 || name.contains("--") {
        return Err("skill name must contain 1..=64 ASCII characters".to_owned());
    }
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(format!(
            "invalid skill name '{name}': first and last characters must be alphanumeric"
        ));
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return Err(format!(
            "invalid skill name '{name}': use lowercase ASCII letters, digits, and hyphens"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, name: &str, description: &str, body: &str) {
        let directory = root.join(name);
        fs::create_dir_all(&directory).expect("skill directory");
        fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {description}\nallowed-tools: Read\n---\n\n{body}\n"
            ),
        )
        .expect("skill file");
    }

    #[test]
    fn project_skill_overrides_user_skill_by_name() {
        let user = tempfile::tempdir().expect("user root");
        let project = tempfile::tempdir().expect("project root");
        write_skill(user.path(), "review", "user review", "user body");
        write_skill(project.path(), "review", "project review", "project body");
        write_skill(user.path(), "testing", "testing help", "testing body");

        let skills = discover_from_roots(Some(user.path()), project.path()).expect("skills");

        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "review");
        assert_eq!(skills[0].body, "project body");
        assert_eq!(skills[0].source, SkillSource::Project);
        assert_eq!(skills[1].name, "testing");
    }

    #[test]
    fn skill_frontmatter_accepts_compatible_extra_fields() {
        let root = tempfile::tempdir().expect("root");
        write_skill(
            root.path(),
            "rust-check",
            "Run Rust checks",
            "Use cargo test.",
        );

        let skills = discover_from_roots(None, root.path()).expect("skills");

        assert_eq!(skills[0].description, "Run Rust checks");
        assert_eq!(skills[0].body, "Use cargo test.");
    }

    #[test]
    fn skill_name_must_match_directory() {
        let root = tempfile::tempdir().expect("root");
        let directory = root.path().join("wrong-directory");
        fs::create_dir_all(&directory).expect("directory");
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: actual-name\ndescription: Test\n---\nBody\n",
        )
        .expect("skill");

        let error = discover_from_roots(None, root.path()).expect_err("mismatch must fail");

        assert!(error.contains("must match directory"), "{error}");
    }
}
