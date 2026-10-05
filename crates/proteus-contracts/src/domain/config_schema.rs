//! Module-owned presentation of opaque configuration. This metadata never
//! grants authority or replaces the implementation's configuration validation.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModuleConfigSchema {
    pub fields: Vec<ConfigField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigField {
    pub key: String,
    pub title: String,
    pub description: String,
    pub value: ConfigValueSchema,
    pub default: Option<Value>,
    pub required: bool,
    pub advanced: bool,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigValueSchema {
    Boolean {},
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Number {},
    String {
        multiline: bool,
        secret: bool,
    },
    Enum {
        options: Vec<ConfigChoice>,
    },
    Array {
        items: Box<ConfigValueSchema>,
    },
    Object {
        fields: Vec<ConfigField>,
    },
    Json {},
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigChoice {
    pub value: Value,
    pub title: String,
}

impl ConfigField {
    pub fn new(key: &str, title: &str, description: &str, value: ConfigValueSchema) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            description: description.into(),
            value,
            default: None,
            required: false,
            advanced: false,
            unit: None,
        }
    }
    pub fn with_default(mut self, value: impl Into<Value>) -> Self {
        self.default = Some(value.into());
        self
    }
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
    pub fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

impl ConfigValueSchema {
    pub fn text() -> Self {
        Self::String {
            multiline: false,
            secret: false,
        }
    }
    pub fn integer(minimum: i64) -> Self {
        Self::Integer {
            minimum: Some(minimum),
            maximum: None,
        }
    }
    pub fn strings() -> Self {
        Self::Array {
            items: Box::new(Self::text()),
        }
    }
    pub fn choices(options: &[(&str, &str)]) -> Self {
        Self::Enum {
            options: options
                .iter()
                .map(|(value, title)| ConfigChoice {
                    value: Value::String((*value).into()),
                    title: (*title).into(),
                })
                .collect(),
        }
    }
}

impl ModuleConfigSchema {
    /// Reject malformed presentation metadata at the process boundary.
    pub fn validate(&self) -> Result<(), String> {
        validate_fields(&self.fields, 0)
    }
}

fn validate_fields(fields: &[ConfigField], depth: usize) -> Result<(), String> {
    let mut keys = std::collections::BTreeSet::new();
    for field in fields {
        if field.key.trim().is_empty() || field.title.trim().is_empty() || !keys.insert(&field.key)
        {
            return Err(format!(
                "config schema has an empty or duplicate field {:?}",
                field.key
            ));
        }
        validate_type(&field.value, depth)?;
        if let Some(value) = &field.default {
            validate_value(&field.value, value)
                .map_err(|error| format!("config schema default for {}: {error}", field.key))?;
        }
    }
    Ok(())
}

fn validate_type(schema: &ConfigValueSchema, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("config schema nesting exceeds 16".into());
    }
    match schema {
        ConfigValueSchema::Integer {
            minimum: Some(min),
            maximum: Some(max),
        } if min > max => return Err("config schema has an inverted integer range".into()),
        ConfigValueSchema::Enum { options } => {
            if options.is_empty() {
                return Err("config schema enum is empty".into());
            }
            for (index, option) in options.iter().enumerate() {
                if option.title.trim().is_empty()
                    || !matches!(
                        option.value,
                        Value::String(_) | Value::Bool(_) | Value::Number(_)
                    )
                    || options[..index]
                        .iter()
                        .any(|other| other.value == option.value)
                {
                    return Err("config schema enum has an invalid or duplicate option".into());
                }
            }
        }
        ConfigValueSchema::Object { fields } => validate_fields(fields, depth + 1)?,
        ConfigValueSchema::Array { items } => validate_type(items, depth + 1)?,
        _ => {}
    }
    Ok(())
}

fn validate_value(schema: &ConfigValueSchema, value: &Value) -> Result<(), String> {
    let valid = match schema {
        ConfigValueSchema::Boolean {} => value.is_boolean(),
        ConfigValueSchema::Integer { minimum, maximum } => value.as_i64().is_some_and(|n| {
            minimum.is_none_or(|min| n >= min) && maximum.is_none_or(|max| n <= max)
        }),
        ConfigValueSchema::Number {} => value.is_number(),
        ConfigValueSchema::String { .. } => value.is_string(),
        ConfigValueSchema::Enum { options } => options.iter().any(|option| option.value == *value),
        ConfigValueSchema::Array { items } => value
            .as_array()
            .is_some_and(|array| array.iter().all(|item| validate_value(items, item).is_ok())),
        ConfigValueSchema::Object { fields } => value.as_object().is_some_and(|object| {
            object.iter().all(|(key, item)| {
                fields
                    .iter()
                    .find(|field| field.key == *key)
                    .is_some_and(|field| validate_value(&field.value, item).is_ok())
            })
        }),
        ConfigValueSchema::Json {} => true,
    };
    if valid {
        Ok(())
    } else {
        Err("value does not match its declared type or range".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_rejects_bad_defaults_duplicates_and_unknown_shapes() {
        let field = ConfigField::new("enabled", "Включено", "", ConfigValueSchema::Boolean {})
            .with_default(true);
        ModuleConfigSchema {
            fields: vec![field.clone()],
        }
        .validate()
        .unwrap();
        assert!(
            ModuleConfigSchema {
                fields: vec![field.clone(), field]
            }
            .validate()
            .is_err()
        );
        let field = ConfigField::new("count", "Количество", "", ConfigValueSchema::integer(1))
            .with_default(0);
        assert!(
            ModuleConfigSchema {
                fields: vec![field]
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_value::<ConfigValueSchema>(
                serde_json::json!({"type":"boolean","legacy":true})
            )
            .is_err()
        );
    }
}
