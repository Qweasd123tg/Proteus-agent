//! Hello-policy-patch plugin.
//!
//! Регистрирует два модуля:
//! - ApprovalPolicy под id `"hello"` — всегда `Ask`, описание `"hello-policy says ask"`.
//! - PatchApplier под id `"hello"` — noop, возвращает `PatchResult { ok: true, summary: "hello-patch noop" }`.
//!
//! Польза ноль, цель — показать что policy/patch slot'ы доступны плагинам.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    plugin::{
        PatchApplierObject, PluginApprovalPolicy, PluginApprovalPolicy_TO, PluginPatchApplier,
        PluginPatchApplier_TO, PluginPatchError, PluginPolicyError, PluginRegisterError,
        PluginRegistryMut, PluginRoot, PluginRoot_Ref, PolicyObject,
    },
};
use serde_json::json;

struct HelloPolicy;

impl PluginApprovalPolicy for HelloPolicy {
    fn evaluate_json(
        &self,
        _call_json: RString,
        _ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError> {
        let decision = json!({
            "Ask": { "reason": "hello-policy says ask" }
        });
        RResult::ROk(RString::from(decision.to_string()))
    }

    fn evaluate_visibility_json(
        &self,
        _ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError> {
        let decision = serde_json::Value::String("Allow".to_string());
        RResult::ROk(RString::from(decision.to_string()))
    }
}

struct HelloPatch;

impl PluginPatchApplier for HelloPatch {
    fn apply_json(
        &self,
        _patch_json: RString,
        _cwd: RString,
    ) -> RResult<RString, PluginPatchError> {
        let result = json!({
            "ok": true,
            "summary": "hello-patch noop"
        });
        RResult::ROk(RString::from(result.to_string()))
    }
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let policy: PolicyObject = PluginApprovalPolicy_TO::from_value(HelloPolicy, TD_Opaque);
    if let RResult::RErr(err) = registry.register_approval_policy(RString::from("hello"), policy) {
        return RResult::RErr(err);
    }

    let patch: PatchApplierObject = PluginPatchApplier_TO::from_value(HelloPatch, TD_Opaque);
    if let RResult::RErr(err) = registry.register_patch_applier(RString::from("hello"), patch) {
        return RResult::RErr(err);
    }

    RResult::ROk(())
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("hello-policy-patch"),
        description: RStr::from_str(
            "Sample plugin: registers 'hello' ApprovalPolicy (Ask) and 'hello' PatchApplier (noop)",
        ),
        register_modules,
    }
    .leak_into_prefix()
}
