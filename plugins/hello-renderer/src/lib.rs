//! Hello-world renderer plugin.
//!
//! Первый настоящий dylib-плагин для modular-agent. Реализует Renderer slot
//! с module_id `"hello"`. После сборки (`cargo build --release -p hello-renderer`)
//! положить `libhello_renderer.so` в `~/.agent/plugins/` и выставить в config:
//!
//! ```toml
//! [modules]
//! renderer = "hello"
//! ```
//!
//! Перезапустить агента — statusline будет из плагина, не из builtin.

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
    contracts::{RenderError, Renderer, Renderer_TO, RendererObject, parse_output_json},
    plugin::{PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref},
};

/// Собственно renderer: оборачивает текст в декоративные рамки.
struct HelloRenderer;

impl Renderer for HelloRenderer {
    fn render_json(&self, output_json: RString) -> RResult<RString, RenderError> {
        let output = match parse_output_json(output_json.as_str()) {
            Ok(output) => output,
            Err(error) => {
                return RResult::RErr(RenderError::new(format!(
                    "failed to parse agent output: {error}"
                )));
            }
        };

        let text = output.text;
        let decorated =
            format!("╔════ hello from plugin ════╗\n{text}\n╚═══════════════════════════╝");
        RResult::ROk(decorated.into())
    }
}

/// Callback, который ядро вызывает после загрузки плагина.
///
/// Плагин создаёт sabi_trait объект и регистрирует его в Registry под
/// module_id `"hello"`.
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let renderer: RendererObject = Renderer_TO::from_value(HelloRenderer, TD_Opaque);
    registry.register_renderer(RString::from("hello"), renderer)
}

/// Экспорт PluginRoot. `#[export_root_module]` — abi_stable attribute,
/// оно генерирует символ, который ядро находит через `lib_header_from_path`.
#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("hello-renderer"),
        description: RStr::from_str("Sample plugin: decorates agent output with a box frame"),
        register_modules,
    }
    .leak_into_prefix()
}
