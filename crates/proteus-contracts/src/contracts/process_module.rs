use serde::{Deserialize, Deserializer, Serialize};

/// Текущая pre-release версия общего stdio component protocol.
///
/// `v3` добавляет multiplexing нескольких одновременных invocation поверх
/// одного persistent component process и строгую invocation-scoped routing.
pub const PROCESS_COMPONENT_PROTOCOL_VERSION: &str = "v3";
pub const PROCESS_COMPONENT_INITIALIZE_METHOD: &str = "initialize";
pub const PROCESS_MODULE_CANCEL_METHOD: &str = "$/cancelRequest";
pub const PROCESS_MODULE_PROGRESS_METHOD: &str = "module.progress";
pub const PROCESS_MODULE_ACTIVITY_METHOD: &str = "module.activity";

/// JSON-RPC error code returned by a worker after cooperative cancellation.
pub const PROCESS_MODULE_CANCELLED_CODE: i64 = -32800;

/// Host-defined cardinality of one process contract surface.
///
/// A module echoes this value during initialization; it cannot negotiate a
/// different composition mode for its own `module_id`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessModuleComposition {
    SelectOne,
    OrderedMany,
}

/// Стабильная identity одного export внутри process component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentExportRef {
    pub slot: String,
    pub module_id: String,
}

impl ProcessComponentExportRef {
    pub fn new(slot: impl Into<String>, module_id: impl Into<String>) -> Self {
        Self {
            slot: slot.into(),
            module_id: module_id.into(),
        }
    }
}

/// Host binding одного export, передаваемый при component initialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentExportInitialize {
    pub slot: String,
    pub module_id: String,
    pub contract_version: String,
    pub composition: ProcessModuleComposition,
    pub module_config: serde_json::Value,
    pub host_features: Vec<String>,
}

impl ProcessComponentExportInitialize {
    pub fn new(
        slot: impl Into<String>,
        module_id: impl Into<String>,
        contract_version: impl Into<String>,
        composition: ProcessModuleComposition,
        module_config: serde_json::Value,
        host_features: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            slot: slot.into(),
            module_id: module_id.into(),
            contract_version: contract_version.into(),
            composition,
            module_config,
            host_features: host_features.into_iter().map(Into::into).collect(),
        }
    }
}

/// Параметры первого JSON-RPC вызова к freshly spawned process component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentInitialize {
    pub protocol_version: String,
    pub component_id: String,
    pub exports: Vec<ProcessComponentExportInitialize>,
}

impl ProcessComponentInitialize {
    pub fn new(
        component_id: impl Into<String>,
        exports: impl IntoIterator<Item = ProcessComponentExportInitialize>,
    ) -> Self {
        Self {
            protocol_version: PROCESS_COMPONENT_PROTOCOL_VERSION.to_owned(),
            component_id: component_id.into(),
            exports: exports.into_iter().collect(),
        }
    }
}

/// Один export, подтверждённый component manifest-ом.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentExportManifest {
    pub slot: String,
    pub module_id: String,
    pub contract_version: String,
    pub composition: ProcessModuleComposition,
    pub module_features: Vec<String>,
}

/// Манифест, который process component возвращает из `initialize`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentManifest {
    pub protocol_version: String,
    pub component_id: String,
    pub exports: Vec<ProcessComponentExportManifest>,
}

/// Host-owned lineage одного module-method invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessInvocationLineage {
    pub root_invocation_id: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub parent_invocation_id: Option<String>,
    pub depth: usize,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

impl ProcessInvocationLineage {
    pub fn root(invocation_id: impl Into<String>) -> Self {
        Self {
            root_invocation_id: invocation_id.into(),
            parent_invocation_id: None,
            depth: 0,
        }
    }
}

/// Обёртка каждого module-method invocation. Target и lineage задаёт host;
/// module получает их только как routing metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentInvocation {
    pub export: ProcessComponentExportRef,
    pub lineage: ProcessInvocationLineage,
    pub params: serde_json::Value,
}

impl ProcessComponentInvocation {
    pub fn new(
        export: ProcessComponentExportRef,
        lineage: ProcessInvocationLineage,
        params: serde_json::Value,
    ) -> Self {
        Self {
            export,
            lineage,
            params,
        }
    }
}

/// Invocation-scoped callback request emitted by a component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleCallbackParams {
    pub invocation_id: String,
    pub params: serde_json::Value,
}

/// Invocation-scoped progress/activity notification emitted by a component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleNotificationParams {
    pub invocation_id: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessModuleCancelCause {
    User,
    Timeout,
    Shutdown,
}

/// Parameters of the invocation-scoped cooperative cancellation notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleCancel {
    pub invocation_id: String,
    pub cause: ProcessModuleCancelCause,
}

impl ProcessModuleCancel {
    pub fn new(invocation_id: impl Into<String>, cause: ProcessModuleCancelCause) -> Self {
        Self {
            invocation_id: invocation_id.into(),
            cause,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_component_handshake_is_strict() {
        let export = ProcessComponentExportInitialize::new(
            "search",
            "python_rg",
            "v1",
            ProcessModuleComposition::SelectOne,
            serde_json::json!({ "roots": ["src"] }),
            ["branch_state"],
        );
        let initialize = ProcessComponentInitialize::new("python-search", [export]);
        let value = serde_json::to_value(&initialize).expect("initialize value");
        assert_eq!(value["protocol_version"], "v3");
        assert_eq!(value["component_id"], "python-search");
        assert_eq!(value["exports"][0]["composition"], "select_one");
        assert_eq!(value["exports"][0]["module_id"], "python_rg");

        let mut unknown = value;
        unknown
            .as_object_mut()
            .expect("initialize object")
            .insert("legacy_version".to_owned(), serde_json::json!(0));
        serde_json::from_value::<ProcessComponentInitialize>(unknown)
            .expect_err("unknown handshake fields must be rejected");
    }

    #[test]
    fn ordered_many_is_explicit_wire_metadata_not_a_module_feature() {
        let initialize = ProcessComponentExportInitialize::new(
            "runtime_contribution",
            "audit",
            "v1",
            ProcessModuleComposition::OrderedMany,
            serde_json::json!({}),
            std::iter::empty::<String>(),
        );

        let value = serde_json::to_value(initialize).expect("initialize value");
        assert_eq!(value["composition"], "ordered_many");
        assert_eq!(value["host_features"], serde_json::json!([]));
    }

    #[test]
    fn component_invocation_carries_target_and_lineage() {
        let call = ProcessComponentInvocation::new(
            ProcessComponentExportRef::new("search", "rg"),
            ProcessInvocationLineage::root("h:1:7"),
            serde_json::json!({"query": "needle"}),
        );

        assert_eq!(
            serde_json::to_value(call).expect("call"),
            serde_json::json!({
                "export": {"slot": "search", "module_id": "rg"},
                "lineage": {
                    "root_invocation_id": "h:1:7",
                    "parent_invocation_id": null,
                    "depth": 0
                },
                "params": {"query": "needle"}
            })
        );

        serde_json::from_value::<ProcessComponentInvocation>(serde_json::json!({
            "export": {"slot": "search", "module_id": "rg"},
            "lineage": {
                "root_invocation_id": "h:1:7",
                "depth": 0
            },
            "params": {"query": "needle"}
        }))
        .expect_err("nullable parent_invocation_id must still be present on wire");
    }

    #[test]
    fn cancel_payload_is_strict() {
        let value = serde_json::to_value(ProcessModuleCancel::new(
            "h:1:7",
            ProcessModuleCancelCause::Timeout,
        ))
        .expect("cancel value");
        assert_eq!(
            value,
            serde_json::json!({ "invocation_id": "h:1:7", "cause": "timeout" })
        );

        serde_json::from_value::<ProcessModuleCancel>(serde_json::json!({
            "invocation_id": "h:1:7",
            "cause": "timeout",
            "request_id": 7
        }))
        .expect_err("unknown cancel fields must be rejected");
    }
}
