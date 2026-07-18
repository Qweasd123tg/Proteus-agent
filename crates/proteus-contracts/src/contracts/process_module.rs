use serde::{Deserialize, Serialize};

/// Текущая внутренняя версия stdio process-module protocol.
pub const PROCESS_MODULE_PROTOCOL_VERSION: &str = "v0";
pub const PROCESS_MODULE_INITIALIZE_METHOD: &str = "initialize";

/// Параметры первого JSON-RPC вызова к freshly spawned process module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleInitialize {
    pub protocol_version: String,
    pub slot: String,
    pub contract_version: String,
}

impl ProcessModuleInitialize {
    pub fn new(slot: impl Into<String>, contract_version: impl Into<String>) -> Self {
        Self {
            protocol_version: PROCESS_MODULE_PROTOCOL_VERSION.to_owned(),
            slot: slot.into(),
            contract_version: contract_version.into(),
        }
    }
}

/// Манифест, который process module возвращает из `initialize`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleManifest {
    pub protocol_version: String,
    pub slot: String,
    pub module_id: String,
    pub contract_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_module_handshake_is_strict() {
        let initialize = ProcessModuleInitialize::new("search", "v0");
        let value = serde_json::to_value(&initialize).expect("initialize value");
        assert_eq!(value["protocol_version"], "v0");

        let mut unknown = value;
        unknown
            .as_object_mut()
            .expect("initialize object")
            .insert("legacy_version".to_owned(), serde_json::json!(0));
        serde_json::from_value::<ProcessModuleInitialize>(unknown)
            .expect_err("unknown handshake fields must be rejected");
    }
}
