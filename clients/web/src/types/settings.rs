use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionMode {
    Plan,
    Normal,
    Auto,
}

impl PermissionMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Normal => "normal",
            Self::Auto => "auto",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Plan => "только чтение",
            Self::Normal => "спрашивать перед записью",
            Self::Auto => "писать без запросов",
        }
    }

    pub(crate) fn from_value(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ReasoningEffort {
    #[default]
    Config,
    Custom(String),
}

impl ReasoningEffort {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Config => "auto".to_owned(),
            Self::Custom(value) => value.clone(),
        }
    }

    pub(crate) fn value(&self) -> String {
        match self {
            Self::Config => "auto".to_owned(),
            Self::Custom(value) => value.clone(),
        }
    }

    pub(crate) fn effort(&self) -> Option<String> {
        match self {
            Self::Config => None,
            Self::Custom(value) => Some(value.clone()),
        }
    }

    pub(crate) fn from_value(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case("auto")
            || value.eq_ignore_ascii_case("config")
        {
            Self::Config
        } else {
            Self::Custom(value.to_owned())
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionToken(Option<String>);

impl SessionToken {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            Self(None)
        } else {
            Self(Some(value.to_owned()))
        }
    }

    pub(crate) fn missing() -> Self {
        Self(None)
    }

    pub(crate) fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}
