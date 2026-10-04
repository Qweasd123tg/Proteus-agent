//! File tools reference process module: read_file, write_file, list_dir, grep, find_files,
//! read_many_files.
//!
//! Reference-реализация файловых tools, экспортируемая единым процессным модулем.
//! Она использует sync `ToolModule` + `std::fs` (не `tokio::fs`) и проверяет,
//! что поведение tools можно вынести за границу core.
//!
//! Этот crate не является шаблоном для новых modules: целевая граница —
//! process protocol из `docs/architecture/process-module-architecture.md`.
//!
//! ## Установка
//!
//! ```bash
//! cargo build --release -p file-tools
//! ```
//!
//! Реализация линкуется только внутрь `proteus-reference-module`; host видит
//! её через общий `tool` process contract.
//!
//! После этого добавьте нужные имена (`read_file`, `write_file`, `list_dir`,
//! `grep`, `find_files`, `read_many_files`) в `tools.enabled`. Установленный
//! module расширяет namespace, но tools остаются opt-in через config.

mod edit;
mod find;
mod list;
mod read;
mod read_many;
mod search;
mod util;
mod write;

use proteus_contracts::process_module::{ModuleRegistry, ProcessModuleError, ToolModuleObject};

use crate::{
    edit::EditFileTool, find::FindFilesTool, list::ListDirTool, read::ReadFileTool,
    read_many::ReadManyFilesTool, search::GrepTool, write::WriteFileTool,
};

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let read: ToolModuleObject = Box::new(ReadFileTool);
    if let Err(err) = registry.register_tool(read) {
        return Err(err);
    }

    let write: ToolModuleObject = Box::new(WriteFileTool);
    if let Err(err) = registry.register_tool(write) {
        return Err(err);
    }

    let edit: ToolModuleObject = Box::new(EditFileTool);
    if let Err(err) = registry.register_tool(edit) {
        return Err(err);
    }

    let list: ToolModuleObject = Box::new(ListDirTool);
    if let Err(err) = registry.register_tool(list) {
        return Err(err);
    }

    let grep: ToolModuleObject = Box::new(GrepTool);
    if let Err(err) = registry.register_tool(grep) {
        return Err(err);
    }

    let find_files: ToolModuleObject = Box::new(FindFilesTool);
    if let Err(err) = registry.register_tool(find_files) {
        return Err(err);
    }

    let read_many: ToolModuleObject = Box::new(ReadManyFilesTool);
    if let Err(err) = registry.register_tool(read_many) {
        return Err(err);
    }

    Ok(())
}

#[cfg(test)]
mod tests;
