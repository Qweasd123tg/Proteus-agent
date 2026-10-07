use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

pub(super) const PLUGIN_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";

#[derive(Deserialize)]
pub(super) struct Manifest {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "author")]
    _author: Option<Author>,
    #[serde(rename = "homepage")]
    _homepage: Option<String>,
    #[serde(rename = "repository")]
    _repository: Option<String>,
    #[serde(rename = "license")]
    _license: Option<String>,
    #[serde(rename = "keywords")]
    _keywords: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Author {
    #[serde(rename = "name")]
    _name: Option<String>,
    #[serde(rename = "email")]
    _email: Option<String>,
    #[serde(rename = "url")]
    _url: Option<String>,
}

pub(super) fn parse(mut value: Value, warnings: &mut Vec<String>) -> Result<Manifest> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("plugin.json must be an object"))?;
    for key in object.keys().cloned().collect::<Vec<_>>() {
        if ![
            "$schema",
            "name",
            "version",
            "description",
            "author",
            "homepage",
            "repository",
            "license",
            "keywords",
            "extensions",
        ]
        .contains(&key.as_str())
        {
            warnings.push(format!("unknown plugin.json field ignored: {key}"));
            object.remove(&key);
        }
    }
    for (key, value) in object.iter() {
        if key != "extensions" && value.is_null() {
            bail!("plugin.json field {key} has an invalid null value");
        }
    }
    if let Some(author) = object.get("author").and_then(Value::as_object) {
        if author.iter().any(|(key, value)| {
            !["name", "email", "url"].contains(&key.as_str()) || !value.is_string()
        }) {
            bail!("invalid plugin author");
        }
    }
    if let Some(extensions) = object.remove("extensions") {
        if !extensions.is_object() {
            warnings.push("non-object plugin extensions ignored".into());
        }
        // No client namespace is implemented. Unimplemented entries are not validated.
    }
    let manifest: Manifest = serde_json::from_value(value)?;
    if manifest.schema != PLUGIN_SCHEMA {
        bail!("unsupported Agent Plugins schema: {}", manifest.schema);
    }
    let name = &manifest.name;
    if name.is_empty()
        || name.len() > 64
        || name.contains("--")
        || name.contains("..")
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.')
    {
        bail!("invalid Agent Plugin name");
    }
    Ok(manifest)
}
