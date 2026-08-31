use std::{any::TypeId, path::Path};

use proteus_core::process_adapters::{ProcessComponentConfig, ProcessExportLaunchConfig};

#[test]
fn process_config_dtos_remain_public() {
    assert_ne!(
        TypeId::of::<ProcessComponentConfig>(),
        TypeId::of::<ProcessExportLaunchConfig>()
    );
}

#[test]
fn implementation_leaf_modules_are_not_public() {
    let lib = include_str!("../src/lib.rs");
    for forbidden in [
        "pub mod adapters;",
        "pub mod stubs;",
        "pub mod tools;",
        "pub use proteus_contracts::{contracts, domain, model_standard};",
    ] {
        assert!(
            !lib.lines().any(|line| line.trim() == forbidden),
            "implementation module became public again: {forbidden}"
        );
    }

    let process_adapters = include_str!("../src/process_adapters/mod.rs");
    for forbidden in [
        "pub mod client;",
        "pub mod memory;",
        "pub mod patch;",
        "pub mod policy;",
        "pub mod search;",
        "pub mod tool;",
        "pub mod workflow;",
        "pub use client::*;",
        "pub use config::*;",
    ] {
        assert!(
            !process_adapters
                .lines()
                .any(|line| line.trim() == forbidden),
            "concrete process adapter became public again: {forbidden}"
        );
    }
}

#[test]
fn core_facade_has_private_submodules_and_explicit_exports() {
    let core = include_str!("../src/core/mod.rs");
    assert!(
        !core.lines().any(|line| line.trim().starts_with("pub mod ")),
        "core submodule tree must stay private behind the facade"
    );
    assert!(
        !core.lines().any(|line| line.trim().ends_with("::*;")),
        "core facade exports must name every public item explicitly"
    );
}

#[test]
fn retired_renderer_slot_has_no_source_or_facade_entrypoint() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for removed in [
        "crates/proteus-contracts/src/contracts/renderer.rs",
        "crates/proteus-core/src/process_adapters/renderer.rs",
        "crates/proteus-core/src/stubs/text_renderer.rs",
        "modules/reference/renderer-pack/Cargo.toml",
    ] {
        assert!(
            !workspace.join(removed).exists(),
            "retired renderer source returned: {removed}"
        );
    }

    for (surface, source) in [
        (
            "contract facade",
            include_str!("../../proteus-contracts/src/contracts/mod.rs"),
        ),
        (
            "module manifest",
            include_str!("../../proteus-contracts/src/domain/module_manifest.rs"),
        ),
        ("config schema", include_str!("../src/core/config.rs")),
        (
            "process adapter facade",
            include_str!("../src/process_adapters/mod.rs"),
        ),
    ] {
        assert!(
            !source.to_ascii_lowercase().contains("renderer"),
            "retired renderer returned through {surface}"
        );
    }
}
