use std::any::TypeId;

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
    for forbidden in ["pub mod adapters;", "pub mod tools;"] {
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
        "pub mod renderer;",
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
