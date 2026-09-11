# AgentControl / Mailbox До A2A

По решению владельца перед началом миграции сохранён полный committed снимок:

- Git tag: `archive/agent-mail-before-a2a-2026-09-11`;
- commit: `d0458dc0`;
- ветка миграции: `codex/a2a-migration`.

Tag сохраняет весь согласованный исходный tree: AgentControl contracts,
mailbox, pending children, process pool/turn/messaging, collaboration tools,
configs, тесты и документацию. Это архив для изучения и возможного извлечения
идей, не дополнительный compatibility/runtime path.

Открыть снимок отдельно, не меняя текущую ветку:

```bash
git worktree add --detach /tmp/proteus-agent-mail-archive archive/agent-mail-before-a2a-2026-09-11
```

Главные пути внутри снимка:

- `crates/proteus-contracts/src/contracts/agent_control.rs`;
- `crates/proteus-core/src/core/agent_control/`;
- `crates/proteus-core/tests/process_agent_control.rs`;
- `crates/proteus-core/tests/process_agent_pool.rs`;
- `docs/architecture/subagents.md`.

Первый A2A endpoint сосуществует с действующим AgentControl до переключения
его process backend. Наличие прежнего backend в начале миграции не означает
обещание поддерживать два межагентных протокола после cutover.
