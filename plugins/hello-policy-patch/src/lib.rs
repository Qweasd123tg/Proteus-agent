//! Hello-policy-patch plugin.
//!
//! Регистрирует три модуля под id `"hello"`:
//! - ApprovalPolicy — всегда `Ask`, описание `"hello-policy says ask"`.
//! - PatchApplier — noop, возвращает `PatchResult { ok: true, summary: "hello-patch noop" }`.
//! - SearchBackend — всегда один chunk с пометкой "hello-search noop".
//! - Optional V2: context_provider и declarative memory_policy под id `"hello"`.
//!
//! Польза ноль, цель — показать что policy/patch/search slot'ы доступны плагинам.

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
        ContextProviderObject, MemoryPolicyObject, PatchApplierObject, PluginApprovalPolicy,
        PluginApprovalPolicy_TO, PluginContextError, PluginContextProvider,
        PluginContextProvider_TO, PluginMemoryPolicy, PluginMemoryPolicy_TO,
        PluginMemoryPolicyError, PluginPatchApplier, PluginPatchApplier_TO, PluginPatchError,
        PluginPolicyError, PluginRegisterError, PluginRegistryMut, PluginRegistryV2Mut, PluginRoot,
        PluginRoot_Ref, PluginSearchBackend, PluginSearchBackend_TO, PluginSearchError,
        PolicyObject, SearchBackendObject,
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

    fn evaluate_visibility_json(&self, _ctx_json: RString) -> RResult<RString, PluginPolicyError> {
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

struct HelloSearch;

impl PluginSearchBackend for HelloSearch {
    fn search_json(&self, _query_json: RString) -> RResult<RString, PluginSearchError> {
        // Возвращаем один chunk с пометкой — этого хватит, чтобы подтвердить
        // что plugin SearchBackend действительно дёргается ядром.
        let chunks = json!([{
            "source": "plugin:hello-search",
            "content": "hello-search noop",
            "score": 1.0,
            "path": null,
            "metadata": {}
        }]);
        RResult::ROk(RString::from(chunks.to_string()))
    }
}

struct HelloContextProvider;

impl PluginContextProvider for HelloContextProvider {
    fn provide_json(&self, _input_json: RString) -> RResult<RString, PluginContextError> {
        let chunks = json!([{
            "source": "plugin:hello-context",
            "content": "hello-context noop",
            "score": 1.0,
            "path": null,
            "metadata": {
                "provider": "hello"
            }
        }]);
        RResult::ROk(RString::from(chunks.to_string()))
    }
}

struct HelloMemoryPolicy;

impl PluginMemoryPolicy for HelloMemoryPolicy {
    fn after_turn_json(&self, _input_json: RString) -> RResult<RString, PluginMemoryPolicyError> {
        let plan = json!({
            "ops": [],
            "metadata": {
                "source": "hello-policy-patch"
            }
        });
        RResult::ROk(RString::from(plan.to_string()))
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

    let search: SearchBackendObject = PluginSearchBackend_TO::from_value(HelloSearch, TD_Opaque);
    if let RResult::RErr(err) = registry.register_search_backend(RString::from("hello"), search) {
        return RResult::RErr(err);
    }

    RResult::ROk(())
}

#[unsafe(no_mangle)]
pub extern "C" fn agent_plugin_register_modules_v2(
    registry: &mut PluginRegistryV2Mut<'_>,
) -> RResult<(), PluginRegisterError> {
    let context: ContextProviderObject =
        PluginContextProvider_TO::from_value(HelloContextProvider, TD_Opaque);
    if let RResult::RErr(err) = registry.register_context_provider(RString::from("hello"), context)
    {
        return RResult::RErr(err);
    }

    let memory_policy: MemoryPolicyObject =
        PluginMemoryPolicy_TO::from_value(HelloMemoryPolicy, TD_Opaque);
    if let RResult::RErr(err) =
        registry.register_memory_policy(RString::from("hello"), memory_policy)
    {
        return RResult::RErr(err);
    }

    RResult::ROk(())
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("hello-policy-patch"),
        description: RStr::from_str(
            "Sample plugin: registers 'hello' ApprovalPolicy (Ask), PatchApplier (noop), and SearchBackend (noop)",
        ),
        register_modules,
    }
    .leak_into_prefix()
}
