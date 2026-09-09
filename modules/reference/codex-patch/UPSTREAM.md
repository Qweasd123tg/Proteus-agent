# Источник Алгоритма

Reference export `patch/codex` адаптирован из OpenAI Codex commit
`67cc3c318dc8b5532db6ade4182b1dc6f3870889`:

- `codex-rs/apply-patch/src/{parser,streaming_parser,seek_sequence}.rs` — parser
  и поиск context; streaming parser и его fixtures перенесены с отделением tests;
- `file_update.rs` — только default-ветка `NormalizeToLf`; `update.rs`
  использует строки и список replacements без exec-server и diff renderer;
- `invocation.rs::try_verify_apply_patch_args` и `lib.rs::apply_hunks_to_files` —
  предварительная проверка, повторное чтение при применении, порядок writes,
  отсутствие rollback после ошибки записи и группировка success summary;
- `core/src/tools/handlers/apply_patch.rs` — prefix ошибок verification и
  отказ при Environment ID в локальном режиме.

[Исходный код](https://github.com/openai/codex/tree/67cc3c318dc8b5532db6ade4182b1dc6f3870889/codex-rs/apply-patch)
распространяется под Apache-2.0; [LICENSE](LICENSE) и [NOTICE](NOTICE) включены
рядом. Перенесённые файлы помечены как адаптированные.

Выбран локальный режим и default `ApplyPatchPreserveLineEndings = false`.
Путь исполнения — общий `patch/v1`: opaque `Patch.content`, workspace cwd,
`PatchResult` или module error. Remote Environment ID отклоняется явно.
Workspace path validation сохраняет существующее ограничение Proteus:
относительные пути без parent traversal и без symlink-компонентов. Это
ограничение текущей локальной сборки, а не полная filesystem semantics Codex.
Полный upstream applied delta, diff-progress events, длительность/exit-code
обёртки tool output и optional PreserveLineEndings не представлены этим срезом.
Proxy-профили сохраняют function tool surface; freeform transport настраивается
отдельно. Синтаксис module задаётся profile instructions, Core его не выбирает.
