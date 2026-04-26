# Modular Agent

Rust CLI-first skeleton for a modular coding agent.

The current implementation is intentionally small:

- stable DTOs in `src/domain` and `src/model_standard`;
- traits in `src/contracts`;
- wiring in `src/core`;
- built-in stub modules in `src/modules`;
- fake model, simple context builder, JSONL event log, read/write/shell/search tools;
- module-swap tests for search, memory, policy, and canonical model contract.

Open the interactive terminal:

```bash
cargo run
```

Install as a command:

```bash
./install.sh
```

Then use it from any workspace:

```bash
cd /path/to/project
agent
```

By default it reads user config from `/home/qweasd123tg/.config/agent-qweasd123tg/config.json` when that file exists.
Sessions are stored next to that config under `sessions/<encoded-cwd>/<session-name|date>/messages.jsonl`.
For example, `/home/game` maps to `sessions/home|game/...`.

Inside the prompt:

```text
agent> read_file Cargo.toml
agent> summarize project
agent> /exit
```

Run one task directly:

```bash
cargo run -- read_file Cargo.toml
```

Run with an explicit config:

```bash
cargo run -- --config agent.example.toml summarize project
```

Use one system JSON config:

```bash
mkdir -p /home/qweasd123tg/.config/agent-qweasd123tg
cp config.example.json /home/qweasd123tg/.config/agent-qweasd123tg/config.json
# edit active_provider, api_key, base_url, and model
cargo run
```

Minimal provider section:

```json
{
  "active_provider": "anthropic",
  "providers": {
    "anthropic": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514",
      "api_key": "sk-ant-...",
      "base_url": "https://api.anthropic.com"
    },
    "local": {
      "provider": "openai_compatible",
      "model": "local-model-name",
      "api_key": "not-needed",
      "base_url": "http://127.0.0.1:11434/v1"
    }
  }
}
```

Run with an explicit JSON config:

```bash
cargo run -- --config config.example.json
```

Validate:

```bash
cargo test
```

The architecture follows `MODULAR_AGENT_SPEC_RU.md`: core code talks to modules through traits and canonical DTOs, not provider SDK types.
